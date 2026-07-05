//! 원격 서비스 기반(Phase 1) 통합 테스트. feature = "mcp-http".
#![cfg(feature = "mcp-http")]

use rhwp::mcp::http::auth;
use rhwp::mcp::http::config::Config;
use rhwp::mcp::http::db::Db;
use rhwp::mcp::http::db::DocumentRow;
use rhwp::mcp::http::store::Store;
use std::sync::Mutex;

// 두 테스트가 프로세스 env(`std::env::set_var`/`remove_var`)를 공유하는데, 카고 테스트는
// 기본적으로 여러 스레드에서 병렬 실행되므로(brief에서 예견한 문제) 뮤텍스로 직렬화해
// 레이스 없이 실행 순서와 무관하게 통과하도록 한다.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn config_from_env_reads_all_fields() {
    let _guard = ENV_LOCK.lock().unwrap();

    std::env::set_var("RHWP_DATABASE_URL", "postgres://u:p@localhost:5433/rhwp");
    std::env::set_var("RHWP_S3_ENDPOINT", "http://localhost:9002");
    std::env::set_var("RHWP_S3_REGION", "us-east-1");
    std::env::set_var("RHWP_S3_BUCKET", "rhwp-docs");
    std::env::set_var("RHWP_S3_ACCESS_KEY", "minioadmin");
    std::env::set_var("RHWP_S3_SECRET_KEY", "minioadmin");
    std::env::set_var("RHWP_S3_PATH_STYLE", "true");

    let c = Config::from_env().expect("config");
    assert_eq!(c.database_url, "postgres://u:p@localhost:5433/rhwp");
    assert_eq!(c.s3_bucket, "rhwp-docs");
    assert!(c.s3_use_path_style);
}

#[test]
fn config_from_env_missing_reports_which() {
    let _guard = ENV_LOCK.lock().unwrap();

    // 다른 필드는 있어도 DB URL 누락이면 에러가 나야 한다.
    std::env::set_var("RHWP_S3_ENDPOINT", "http://localhost:9002");
    std::env::set_var("RHWP_S3_BUCKET", "rhwp-docs");
    std::env::set_var("RHWP_S3_ACCESS_KEY", "minioadmin");
    std::env::set_var("RHWP_S3_SECRET_KEY", "minioadmin");
    std::env::remove_var("RHWP_DATABASE_URL");

    let err = Config::from_env().err().expect("DB URL 누락은 에러여야 함");
    assert!(err.contains("RHWP_DATABASE_URL"), "에러 메시지에 변수명 포함: {err}");
}

fn dev_config() -> Config {
    Config {
        database_url: "postgres://rhwp:rhwp@localhost:5433/rhwp".into(),
        s3_endpoint: "http://localhost:9002".into(),
        s3_region: "us-east-1".into(),
        s3_bucket: "rhwp-docs".into(),
        s3_access_key: "minioadmin".into(),
        s3_secret_key: "minioadmin".into(),
        s3_use_path_style: true,
        cookie_secure: false,
        public_base_url: "http://localhost:8300".into(),
        auth_required: true,
    }
}

#[tokio::test]
async fn db_migrate_and_create_user_roundtrip() {
    let cfg = dev_config();
    let db = match Db::connect(&cfg).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("dev DB 미기동으로 skip: {e}");
            return; // dev 스택 없으면 통과(로컬 편의). CI에서는 스택 기동 후 실행.
        }
    };
    db.migrate().await.expect("migrate");
    let name = format!("u_{}", uuid::Uuid::new_v4());
    let id = db.create_user(&name, "hash", false).await.expect("create");
    let found = db.find_user_by_name(&name).await.expect("find").expect("some");
    assert_eq!(found.0, id);
    assert_eq!(found.1, "hash");
}

#[tokio::test]
async fn set_password_updates_hash() {
    let cfg = dev_config();
    let db = match Db::connect(&cfg).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    db.migrate().await.expect("migrate");
    let name = format!("pw_{}", uuid::Uuid::new_v4());
    let old = auth::hash_secret("oldpw").unwrap();
    db.create_user(&name, &old, false).await.expect("create");
    // 새 비밀번호로 갱신
    let new = auth::hash_secret("newpw").unwrap();
    let n = db.set_password(&name, &new).await.expect("set");
    assert_eq!(n, 1);
    let found = db.find_user_by_name(&name).await.unwrap().unwrap();
    assert!(auth::verify_secret("newpw", &found.1), "새 비번 검증 통과");
    assert!(!auth::verify_secret("oldpw", &found.1), "옛 비번 무효");
    // 없는 사용자 → 0
    let z = db.set_password("nobody-xyz-does-not-exist", &new).await.unwrap();
    assert_eq!(z, 0);
}

#[tokio::test]
async fn store_put_get_and_presign() {
    let cfg = dev_config();
    let store = match Store::connect(&cfg).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dev MinIO 미기동으로 skip: {e}");
            return;
        }
    };
    let key = format!("test/{}.bin", uuid::Uuid::new_v4());
    let data = b"hello-rhwp-\x00\x01\x02";
    let etag = store.put(&key, data).await.expect("put");
    assert!(!etag.is_empty());
    let got = store.get(&key).await.expect("get");
    assert_eq!(got, data);
    let url = store.presigned_get(&key, 300).await.expect("presign get");
    assert!(url.starts_with("http"));
    let purl = store
        .presigned_put(&format!("test/{}.bin", uuid::Uuid::new_v4()), 300)
        .await
        .expect("presign put");
    assert!(purl.starts_with("http"));
}

#[test]
fn token_hash_and_verify() {
    let t = auth::generate_token();
    assert!(t.len() >= 40); // base64url(32B) 대략 43자
    let h = auth::hash_secret(&t).expect("hash");
    assert!(auth::verify_secret(&t, &h));
    assert!(!auth::verify_secret("wrong", &h));
}

#[tokio::test]
async fn authenticate_issued_token() {
    let cfg = dev_config();
    let db = match Db::connect(&cfg).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    db.migrate().await.unwrap();
    let uname = format!("u_{}", uuid::Uuid::new_v4());
    let uid = db.create_user(&uname, "pw", false).await.unwrap();
    let token = auth::generate_token();
    db.issue_token(uid, &auth::hash_secret(&token).unwrap()).await.unwrap();
    let who = db.authenticate_token(&token).await.unwrap();
    assert_eq!(who, Some(uid));
    assert_eq!(db.authenticate_token("nope").await.unwrap(), None);
}

#[tokio::test]
async fn store_get_missing_key_errors() {
    let cfg = dev_config();
    let store = match Store::connect(&cfg).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dev MinIO 미기동으로 skip: {e}");
            return;
        }
    };
    let missing = format!("test/nope-{}.bin", uuid::Uuid::new_v4());
    assert!(
        store.get(&missing).await.is_err(),
        "존재하지 않는 키는 Err 여야 함"
    );
}

#[tokio::test]
async fn document_crud_scoped_to_user() {
    let cfg = dev_config();
    let db = match Db::connect(&cfg).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    db.migrate().await.unwrap();
    let u1 = db.create_user(&format!("a_{}", uuid::Uuid::new_v4()), "p", false).await.unwrap();
    let u2 = db.create_user(&format!("b_{}", uuid::Uuid::new_v4()), "p", false).await.unwrap();
    let h = format!("doc-{}", uuid::Uuid::new_v4());
    let id = db.create_document(u1, &h, "계약서", &format!("users/{u1}/{h}"), "hwp").await.unwrap();
    db.update_document_etag(id, "etag-1").await.unwrap();

    let got: Option<DocumentRow> = db.get_document_by_handle(u1, &h).await.unwrap();
    assert_eq!(got.as_ref().unwrap().etag, "etag-1");
    // 타 사용자에겐 안 보임
    assert!(db.get_document_by_handle(u2, &h).await.unwrap().is_none());
    assert_eq!(db.list_documents(u1).await.unwrap().len(), 1);
    assert_eq!(db.list_documents(u2).await.unwrap().len(), 0);
}
