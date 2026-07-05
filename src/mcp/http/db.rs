//! Postgres 접근(풀, 마이그레이션, 도메인 CRUD).

use deadpool_postgres::{Config as PgConfig, Pool, Runtime};
use tokio_postgres::NoTls;
use uuid::Uuid;

use super::config::Config;

/// Postgres 풀 래퍼.
pub struct Db {
    pub pool: Pool,
}

/// users 목록 행(이름 + 관리자 여부).
pub struct UserRow {
    pub name: String,
    pub is_admin: bool,
}

/// documents 행.
pub struct DocumentRow {
    pub id: Uuid,
    pub handle: String,
    pub name: String,
    pub storage_key: String,
    pub etag: String,
    pub format: String,
}

impl Db {
    /// 설정의 database_url로 풀을 만든다.
    pub async fn connect(cfg: &Config) -> Result<Db, String> {
        let mut pg = PgConfig::new();
        pg.url = Some(cfg.database_url.clone());
        let pool = pg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("풀 생성 실패: {e}"))?;
        // 연결 확인
        let c = pool.get().await.map_err(|e| format!("DB 연결 실패: {e}"))?;
        c.simple_query("SELECT 1").await.map_err(|e| format!("DB 핑 실패: {e}"))?;
        Ok(Db { pool })
    }

    /// 0001_init.sql, 0002_oauth.sql을 순서대로 적용한다(멱등: CREATE TABLE IF NOT EXISTS).
    pub async fn migrate(&self) -> Result<(), String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        c.batch_execute(include_str!("migrations/0001_init.sql"))
            .await
            .map_err(|e| format!("마이그레이션 0001 실패: {e}"))?;
        c.batch_execute(include_str!("migrations/0002_oauth.sql"))
            .await
            .map_err(|e| format!("마이그레이션 0002 실패: {e}"))?;
        c.batch_execute(include_str!("migrations/0003_admin.sql"))
            .await
            .map_err(|e| format!("마이그레이션 0003 실패: {e}"))?;
        Ok(())
    }

    pub async fn create_user(
        &self,
        name: &str,
        pw_hash: &str,
        is_admin: bool,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO users (id, name, pw_hash, is_admin) VALUES ($1, $2, $3, $4)",
            &[&id, &name, &pw_hash, &is_admin],
        )
        .await
        .map_err(|e| format!("사용자 생성 실패: {e}"))?;
        Ok(id)
    }

    /// 사용자 목록(이름, 관리자 여부). 생성순 정렬.
    pub async fn list_users(&self) -> Result<Vec<UserRow>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = c
            .query(
                "SELECT name, is_admin FROM users ORDER BY created_at",
                &[],
            )
            .await
            .map_err(|e| format!("사용자 목록 실패: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|r| UserRow {
                name: r.get(0),
                is_admin: r.get(1),
            })
            .collect())
    }

    /// 사용자 비밀번호(pw_hash)를 갱신한다. 반환: 영향받은 행 수(0이면 해당 이름의 사용자 없음).
    pub async fn set_password(&self, name: &str, pw_hash: &str) -> Result<u64, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let n = c
            .execute("UPDATE users SET pw_hash=$2 WHERE name=$1", &[&name, &pw_hash])
            .await
            .map_err(|e| format!("비밀번호 갱신 실패: {e}"))?;
        Ok(n)
    }

    /// 해당 사용자가 관리자인지 여부. 사용자가 없으면 false.
    pub async fn is_admin(&self, user_id: Uuid) -> Result<bool, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = c
            .query_opt("SELECT is_admin FROM users WHERE id=$1", &[&user_id])
            .await
            .map_err(|e| format!("권한 조회 실패: {e}"))?;
        Ok(row.map(|r| r.get::<_, bool>(0)).unwrap_or(false))
    }

    pub async fn find_user_by_name(&self, name: &str) -> Result<Option<(Uuid, String)>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = c
            .query_opt("SELECT id, pw_hash FROM users WHERE name = $1", &[&name])
            .await
            .map_err(|e| format!("사용자 조회 실패: {e}"))?;
        Ok(row.map(|r| (r.get::<_, Uuid>(0), r.get::<_, String>(1))))
    }

    /// 인증 off(자가호스팅) 모드에서 모든 요청이 귀속될 기본 로컬 사용자를 확보한다.
    /// 이름 `__local__` 로 조회하고 없으면 생성한다(비밀번호 해시는 사용되지 않는 더미).
    /// documents.user_id 가 users(id) 를 FK 참조하므로 실재 사용자 행이 필요하다.
    pub async fn ensure_local_user(&self) -> Result<Uuid, String> {
        if let Some((uid, _)) = self.find_user_by_name("__local__").await? {
            return Ok(uid);
        }
        // 사용되지 않는 더미 해시(로그인 경로가 없음 — 인증 off 모드 전용).
        let dummy = super::auth::hash_secret(&Uuid::new_v4().to_string())
            .map_err(|_| "기본 사용자 해시 생성 실패".to_string())?;
        match self.create_user("__local__", &dummy, false).await {
            Ok(uid) => Ok(uid),
            // 동시 기동 경합으로 이미 생성됐으면 재조회.
            Err(_) => self
                .find_user_by_name("__local__")
                .await?
                .map(|(uid, _)| uid)
                .ok_or_else(|| "기본 사용자 확보 실패".to_string()),
        }
    }

    pub async fn issue_token(&self, user_id: Uuid, token_hash: &str) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO api_tokens (id, user_id, token_hash) VALUES ($1,$2,$3)",
            &[&id, &user_id, &token_hash],
        )
        .await
        .map_err(|e| format!("토큰 발급 실패: {e}"))?;
        Ok(id)
    }

    /// 제시된 평문 토큰을 미폐기·미만료 토큰들과 대조하여 user_id를 찾는다.
    /// expires_at이 NULL인 토큰(기존 발급분, issue_token)은 만료 없이 계속 유효하다.
    pub async fn authenticate_token(&self, presented: &str) -> Result<Option<Uuid>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = c
            .query(
                "SELECT user_id, token_hash FROM api_tokens WHERE revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())",
                &[],
            )
            .await
            .map_err(|e| format!("토큰 조회 실패: {e}"))?;
        for r in rows {
            let uid: Uuid = r.get(0);
            let hash: String = r.get(1);
            if super::auth::verify_secret(presented, &hash) {
                return Ok(Some(uid));
            }
        }
        Ok(None)
    }

    pub async fn create_document(
        &self,
        user_id: Uuid,
        handle: &str,
        name: &str,
        storage_key: &str,
        format: &str,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO documents (id,user_id,handle,name,storage_key,format) VALUES ($1,$2,$3,$4,$5,$6)",
            &[&id, &user_id, &handle, &name, &storage_key, &format],
        )
        .await
        .map_err(|e| format!("문서 생성 실패: {e}"))?;
        Ok(id)
    }

    pub async fn get_document_by_handle(
        &self,
        user_id: Uuid,
        handle: &str,
    ) -> Result<Option<DocumentRow>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = c
            .query_opt(
                "SELECT id,handle,name,storage_key,etag,format FROM documents WHERE user_id=$1 AND handle=$2",
                &[&user_id, &handle],
            )
            .await
            .map_err(|e| format!("문서 조회 실패: {e}"))?;
        Ok(row.map(|r| DocumentRow {
            id: r.get(0),
            handle: r.get(1),
            name: r.get(2),
            storage_key: r.get(3),
            etag: r.get(4),
            format: r.get(5),
        }))
    }

    pub async fn list_documents(&self, user_id: Uuid) -> Result<Vec<DocumentRow>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let rows = c
            .query(
                "SELECT id,handle,name,storage_key,etag,format FROM documents WHERE user_id=$1 ORDER BY updated_at DESC",
                &[&user_id],
            )
            .await
            .map_err(|e| format!("문서 목록 실패: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|r| DocumentRow {
                id: r.get(0),
                handle: r.get(1),
                name: r.get(2),
                storage_key: r.get(3),
                etag: r.get(4),
                format: r.get(5),
            })
            .collect())
    }

    /// 주의: 이 함수는 user_id로 소유권을 검증하지 않는다. 호출자가 사전에
    /// `get_document_by_handle(user_id, handle)` 등으로 소유권을 확인한 뒤에만 호출해야 한다.
    pub async fn update_document_etag(&self, id: Uuid, etag: &str) -> Result<(), String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        c.execute(
            "UPDATE documents SET etag=$2, updated_at=now() WHERE id=$1",
            &[&id, &etag],
        )
        .await
        .map_err(|e| format!("etag 갱신 실패: {e}"))?;
        Ok(())
    }

    /// 사용자의 모든 미폐기 토큰을 폐기한다. 반환: 폐기된 개수.
    pub async fn revoke_user_tokens(&self, user_id: Uuid) -> Result<u64, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let n = c.execute(
            "UPDATE api_tokens SET revoked_at = now() WHERE user_id=$1 AND revoked_at IS NULL",
            &[&user_id],
        ).await.map_err(|e| format!("토큰 폐기 실패: {e}"))?;
        Ok(n)
    }

    /// OAuth 동적 클라이언트 등록(DCR). redirect_uris_json은 JSON 배열 문자열.
    pub async fn create_oauth_client(&self, redirect_uris_json: &str, name: &str) -> Result<String, String> {
        let client_id = super::auth::generate_token();
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO oauth_clients (client_id, redirect_uris, client_name) VALUES ($1,$2,$3)",
            &[&client_id, &redirect_uris_json, &name],
        )
        .await
        .map_err(|e| format!("클라이언트 등록 실패: {e}"))?;
        Ok(client_id)
    }

    /// client_id로 등록된 redirect_uris(JSON 문자열)를 조회한다.
    pub async fn get_oauth_client_redirects(&self, client_id: &str) -> Result<Option<String>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = c
            .query_opt("SELECT redirect_uris FROM oauth_clients WHERE client_id=$1", &[&client_id])
            .await
            .map_err(|e| format!("클라이언트 조회 실패: {e}"))?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }

    /// client_id로 등록된 client_name(동의 화면 표시용)을 조회한다.
    pub async fn get_oauth_client_name(&self, client_id: &str) -> Result<Option<String>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = c
            .query_opt("SELECT client_name FROM oauth_clients WHERE client_id=$1", &[&client_id])
            .await
            .map_err(|e| format!("클라이언트 조회 실패: {e}"))?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }

    /// PKCE 인가 코드를 저장한다(code_hash는 SHA-256, ttl_secs 후 만료).
    pub async fn insert_oauth_code(
        &self,
        code_hash: &str,
        client_id: &str,
        user_id: Uuid,
        redirect_uri: &str,
        code_challenge: &str,
        resource: Option<&str>,
        ttl_secs: i64,
    ) -> Result<(), String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO oauth_codes (code_hash,client_id,user_id,redirect_uri,code_challenge,resource,expires_at) VALUES ($1,$2,$3,$4,$5,$6, now()+ ($7 || ' seconds')::interval)",
            &[&code_hash, &client_id, &user_id, &redirect_uri, &code_challenge, &resource, &ttl_secs.to_string()],
        )
        .await
        .map_err(|e| format!("코드 저장 실패: {e}"))?;
        Ok(())
    }

    /// 만료 시각을 지정하는 토큰 발급(OAuth 액세스 토큰용). ttl_secs가 None이면 만료 없음.
    pub async fn issue_token_expiring(
        &self,
        user_id: Uuid,
        token_hash: &str,
        ttl_secs: Option<i64>,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        match ttl_secs {
            Some(t) => {
                c.execute(
                    "INSERT INTO api_tokens (id,user_id,token_hash,expires_at) VALUES ($1,$2,$3, now()+($4||' seconds')::interval)",
                    &[&id, &user_id, &token_hash, &t.to_string()],
                )
                .await
            }
            None => {
                c.execute(
                    "INSERT INTO api_tokens (id,user_id,token_hash) VALUES ($1,$2,$3)",
                    &[&id, &user_id, &token_hash],
                )
                .await
            }
        }
        .map_err(|e| format!("토큰 발급 실패: {e}"))?;
        Ok(id)
    }

    /// 코드를 1회 소비(used=true)하며 유효성 검사. 반환: (user_id, redirect_uri, code_challenge, client_id)
    pub async fn take_valid_code(
        &self,
        code_hash: &str,
    ) -> Result<Option<(Uuid, String, String, String)>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = c
            .query_opt(
                "UPDATE oauth_codes SET used=true WHERE code_hash=$1 AND used=false AND expires_at>now() RETURNING user_id,redirect_uri,code_challenge,client_id",
                &[&code_hash],
            )
            .await
            .map_err(|e| format!("코드 소비 실패: {e}"))?;
        Ok(row.map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))))
    }

    /// 리프레시 토큰을 저장한다(token_hash는 SHA-256, ttl_secs 후 만료).
    pub async fn insert_refresh(
        &self,
        token_hash: &str,
        user_id: Uuid,
        client_id: &str,
        ttl_secs: i64,
    ) -> Result<(), String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO oauth_refresh (token_hash,user_id,client_id,expires_at) VALUES ($1,$2,$3, now()+($4||' seconds')::interval)",
            &[&token_hash, &user_id, &client_id, &ttl_secs.to_string()],
        )
        .await
        .map_err(|e| format!("리프레시 저장 실패: {e}"))?;
        Ok(())
    }

    /// 리프레시 회전: 유효하면 폐기하고 (user_id,client_id) 반환.
    pub async fn consume_refresh(&self, token_hash: &str) -> Result<Option<(Uuid, String)>, String> {
        let c = self.pool.get().await.map_err(|e| e.to_string())?;
        let row = c
            .query_opt(
                "UPDATE oauth_refresh SET revoked_at=now() WHERE token_hash=$1 AND revoked_at IS NULL AND expires_at>now() RETURNING user_id,client_id",
                &[&token_hash],
            )
            .await
            .map_err(|e| format!("리프레시 소비 실패: {e}"))?;
        Ok(row.map(|r| (r.get(0), r.get(1))))
    }
}
