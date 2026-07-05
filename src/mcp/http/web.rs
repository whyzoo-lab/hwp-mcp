//! 웹 컴패니언: 브라우저 쿠키 세션 + /api 핸들러 + 정적 페이지.
use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use super::auth;
use super::doc_ctx;
use super::server::AppState;

/// 쿠키값 → user_id 인메모리 세션(단일 레플리카 전제).
#[derive(Default)]
pub struct WebSessions {
    inner: Mutex<HashMap<String, Uuid>>,
}

impl WebSessions {
    pub fn new() -> Self { WebSessions { inner: Mutex::new(HashMap::new()) } }
    /// 새 세션을 만들고 쿠키값(랜덤 토큰)을 반환.
    pub fn create(&self, user: Uuid) -> String {
        let token = auth::generate_token();
        self.inner.lock().unwrap().insert(token.clone(), user);
        token
    }
    pub fn user_of(&self, cookie_val: &str) -> Option<Uuid> {
        self.inner.lock().unwrap().get(cookie_val).copied()
    }
    pub fn remove(&self, cookie_val: &str) {
        self.inner.lock().unwrap().remove(cookie_val);
    }
}

/// 컴패니언 단일 페이지(정적 HTML).
pub async fn index() -> impl IntoResponse {
    axum::response::Html(include_str!("companion.html"))
}

/// Cookie 헤더에서 rhwp_session 쿠키의 원문 값을 추출한다.
pub fn cookie_value(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("cookie").and_then(|v| v.to_str().ok())?;
    for part in raw.split(';') {
        if let Some(val) = part.trim().strip_prefix("rhwp_session=") {
            return Some(val.to_string());
        }
    }
    None
}

/// Cookie 헤더에서 rhwp_session 값을 파싱해 user_id를 찾는다.
pub fn cookie_user(state: &AppState, headers: &HeaderMap) -> Option<Uuid> {
    let val = cookie_value(headers)?;
    state.web_sessions.user_of(&val)
}

pub async fn login(State(state): State<AppState>, body: String) -> impl IntoResponse {
    let v: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(json!({"error":"잘못된 요청"}))).into_response(),
    };
    let username = v.get("username").and_then(|x| x.as_str()).unwrap_or("");
    let password = v.get("password").and_then(|x| x.as_str()).unwrap_or("");
    let found = match state.db.find_user_by_name(username).await {
        Ok(f) => f,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error":"서버 오류"}))).into_response(),
    };
    let (uid, pw_hash) = match found {
        Some(x) => x,
        None => return (StatusCode::UNAUTHORIZED, Json(json!({"error":"인증 실패"}))).into_response(),
    };
    if !auth::verify_secret(password, &pw_hash) {
        eprintln!("웹 로그인 실패: 비밀번호 불일치");
        return (StatusCode::UNAUTHORIZED, Json(json!({"error":"인증 실패"}))).into_response();
    }
    let cookie = state.web_sessions.create(uid);
    let mut headers = HeaderMap::new();
    let secure = if state.cookie_secure { "; Secure" } else { "" };
    let set = format!("rhwp_session={cookie}; HttpOnly; SameSite=Lax; Path=/{secure}");
    headers.insert("set-cookie", set.parse().unwrap());
    (StatusCode::OK, headers, Json(json!({"ok":true}))).into_response()
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(raw) = headers.get("cookie").and_then(|v| v.to_str().ok()) {
        for part in raw.split(';') {
            if let Some(val) = part.trim().strip_prefix("rhwp_session=") {
                state.web_sessions.remove(val);
            }
        }
    }
    let mut h = HeaderMap::new();
    let secure = if state.cookie_secure { "; Secure" } else { "" };
    let clear = format!("rhwp_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{secure}");
    h.insert("set-cookie", clear.parse().unwrap());
    (StatusCode::OK, h, Json(json!({"ok":true}))).into_response()
}

/// 기존 토큰을 모두 폐기하고 새 베어러 토큰을 1회 반환한다.
pub async fn rotate_token(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let uid = match cookie_user(&state, &headers) {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, Json(json!({"error":"로그인 필요"}))).into_response(),
    };
    if let Err(e) = state.db.revoke_user_tokens(uid).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response();
    }
    let token = auth::generate_token();
    let hash = match auth::hash_secret(&token) {
        Ok(h) => h,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    };
    if let Err(e) = state.db.issue_token(uid, &hash).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response();
    }
    (StatusCode::OK, Json(json!({"token": token}))).into_response()
}

pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let uid = match cookie_user(&state, &headers) {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, Json(json!({"error":"로그인 필요"}))).into_response(),
    };
    // username + 관리자 여부 조회(간단히 pool 사용)
    let (name, is_admin): (String, bool) = match state.db.pool.get().await {
        Ok(c) => match c
            .query_opt("SELECT name, is_admin FROM users WHERE id=$1", &[&uid])
            .await
        {
            Ok(Some(r)) => (r.get::<_, String>(0), r.get::<_, bool>(1)),
            _ => (String::new(), false),
        },
        Err(_) => (String::new(), false),
    };
    (
        StatusCode::OK,
        Json(json!({"user_id": uid.to_string(), "username": name, "is_admin": is_admin})),
    )
        .into_response()
}

/// 로그인 + 관리자 권한을 확인한다. 관리자면 uid, 아니면 에러 응답을 반환한다.
async fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Uuid, axum::response::Response> {
    let uid = cookie_user(state, headers).ok_or_else(|| {
        (StatusCode::UNAUTHORIZED, Json(json!({"error":"로그인 필요"}))).into_response()
    })?;
    match state.db.is_admin(uid).await {
        Ok(true) => Ok(uid),
        Ok(false) => Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error":"관리자 권한이 필요합니다"})),
        )
            .into_response()),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response()),
    }
}

/// (관리자 전용) 사용자 목록.
pub async fn admin_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&state, &headers).await {
        return resp;
    }
    match state.db.list_users().await {
        Ok(users) => {
            let list: Vec<Value> = users
                .into_iter()
                .map(|u| json!({"username": u.name, "is_admin": u.is_admin}))
                .collect();
            (StatusCode::OK, Json(json!({"users": list}))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// (관리자 전용) 계정 생성. body: {username, password, is_admin?}
pub async fn admin_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&state, &headers).await {
        return resp;
    }
    let v: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error":"잘못된 요청"}))).into_response()
        }
    };
    let username = v.get("username").and_then(|x| x.as_str()).unwrap_or("").trim();
    let password = v.get("password").and_then(|x| x.as_str()).unwrap_or("");
    let is_admin = v.get("is_admin").and_then(|x| x.as_bool()).unwrap_or(false);
    if username.is_empty() || password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"아이디와 비밀번호를 모두 입력하세요"})),
        )
            .into_response();
    }
    // 중복 아이디 사전 확인(명확한 에러 메시지).
    match state.db.find_user_by_name(username).await {
        Ok(Some(_)) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({"error":"이미 존재하는 아이디입니다"})),
            )
                .into_response()
        }
        Ok(None) => {}
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
    let hash = match auth::hash_secret(password) {
        Ok(h) => h,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    };
    match state.db.create_user(username, &hash, is_admin).await {
        Ok(id) => (
            StatusCode::OK,
            Json(json!({"ok": true, "user_id": id.to_string(), "username": username, "is_admin": is_admin})),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// 업로드용 앱 프록시 PUT URL을 발급하고 documents 행을 등록한다(바이트는 이후 PUT으로 채워짐).
pub async fn upload_url(State(state): State<AppState>, headers: HeaderMap, body: String) -> impl IntoResponse {
    let uid = match cookie_user(&state, &headers) {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, Json(json!({"error":"로그인 필요"}))).into_response(),
    };
    let v: Value = serde_json::from_str(&body).unwrap_or(json!({}));
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("문서");
    let doc_id = Uuid::new_v4();
    let key = doc_ctx::storage_key(uid, doc_id);
    let handle = doc_ctx::new_handle(name);
    if let Err(e) = state.db.create_document(uid, &handle, name, &key, "hwp").await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("등록 실패: {e}")}))).into_response();
    }
    // 앱 프록시 업로드 링크(외부/브라우저에서 PUT 가능, minio 비노출).
    let token = state.uploads.issue(&key, 3600);
    let base = state.public_base_url.trim_end_matches('/');
    let url = format!("{base}/upload/{token}");
    (StatusCode::OK, Json(json!({"handle": handle, "upload_url": url, "expires_secs": 3600}))).into_response()
}

/// 내 문서 목록.
pub async fn list_docs(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let uid = match cookie_user(&state, &headers) {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, Json(json!({"error":"로그인 필요"}))).into_response(),
    };
    match state.db.list_documents(uid).await {
        Ok(rows) => {
            let docs: Vec<Value> = rows.into_iter()
                .map(|r| json!({"handle": r.handle, "name": r.name, "format": r.format}))
                .collect();
            (StatusCode::OK, Json(json!({"documents": docs}))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// 다운로드용 presigned GET URL(?handle=).
pub async fn download_url(State(state): State<AppState>, headers: HeaderMap, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> impl IntoResponse {
    let uid = match cookie_user(&state, &headers) {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, Json(json!({"error":"로그인 필요"}))).into_response(),
    };
    let handle = match q.get("handle") {
        Some(h) => h,
        None => return (StatusCode::BAD_REQUEST, Json(json!({"error":"handle 필요"}))).into_response(),
    };
    let row = match state.db.get_document_by_handle(uid, handle).await {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({"error":"문서 없음"}))).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    };
    // 앱 프록시 다운로드 링크(외부에서 열림, minio 비노출).
    let filename = format!("{}.{}", row.name, row.format);
    // 사용자가 나중에 열어볼 수 있도록 24시간 유효(인메모리라 재배포 시에는 초기화됨).
    let token = state.downloads.issue(&row.storage_key, &filename, 86_400);
    let base = state.public_base_url.trim_end_matches('/');
    let url = format!("{base}/download/{token}");
    (StatusCode::OK, Json(json!({"download_url": url, "expires_secs": 86_400}))).into_response()
}
