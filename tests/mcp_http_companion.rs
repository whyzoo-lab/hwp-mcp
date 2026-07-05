//! Phase 3 웹 컴패니언 API 통합 테스트. feature = "mcp-http".
#![cfg(feature = "mcp-http")]

use rhwp::mcp::http::config::Config;
use rhwp::mcp::http::server::{build_router, AppState};

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

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

async fn test_state() -> Option<AppState> {
    AppState::connect(&dev_config()).await.ok()
}

/// (status, set-cookie 헤더값 옵션, body) 반환.
async fn post(app: axum::Router, path: &str, body: &str, cookie: Option<&str>) -> (u16, Option<String>, String) {
    let mut b = Request::builder().method("POST").uri(path).header("content-type", "application/json");
    if let Some(c) = cookie { b = b.header("cookie", c); }
    let resp = app.oneshot(b.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let code = resp.status().as_u16();
    let sc = resp.headers().get("set-cookie").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (code, sc, String::from_utf8_lossy(&bytes).to_string())
}

async fn get(app: axum::Router, path: &str, cookie: Option<&str>) -> (u16, String) {
    let mut b = Request::builder().uri(path);
    if let Some(c) = cookie { b = b.header("cookie", c); }
    let resp = app.oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let code = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (code, String::from_utf8_lossy(&bytes).to_string())
}

async fn make_user(state: &AppState, pw: &str) -> String {
    state.db.migrate().await.unwrap();
    let uname = format!("web_{}", uuid::Uuid::new_v4());
    let hash = rhwp::mcp::http::auth::hash_secret(pw).unwrap();
    state.db.create_user(&uname, &hash, false).await.unwrap();
    uname
}

async fn make_admin(state: &AppState, pw: &str) -> String {
    state.db.migrate().await.unwrap();
    let uname = format!("adm_{}", uuid::Uuid::new_v4());
    let hash = rhwp::mcp::http::auth::hash_secret(pw).unwrap();
    state.db.create_user(&uname, &hash, true).await.unwrap();
    uname
}

/// set-cookie 헤더에서 `rhwp_session=...` 쿠키 문자열(이름=값)만 추출.
fn extract_cookie(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap_or("").to_string()
}

#[tokio::test]
async fn login_sets_cookie_and_me_works() {
    let state = match test_state().await { Some(s)=>s, None=>{eprintln!("skip: dev 스택");return;} };
    let uname = make_user(&state, "pw1234").await;
    let app = build_router(state.clone());

    // 잘못된 비번 → 401
    let bad = post(app.clone(), "/api/login", &serde_json::json!({"username":uname,"password":"nope"}).to_string(), None).await;
    assert_eq!(bad.0, 401);

    // 올바른 비번 → 200 + Set-Cookie
    let ok = post(app.clone(), "/api/login", &serde_json::json!({"username":uname,"password":"pw1234"}).to_string(), None).await;
    assert_eq!(ok.0, 200);
    let cookie = extract_cookie(&ok.1.expect("set-cookie"));
    assert!(cookie.starts_with("rhwp_session="));

    // 쿠키로 /api/me → 200, username 반환
    let me = get(app.clone(), "/api/me", Some(&cookie)).await;
    assert_eq!(me.0, 200);
    assert!(me.1.contains(&uname));

    // 쿠키 없이 /api/me → 401
    let no = get(app, "/api/me", None).await;
    assert_eq!(no.0, 401);
}

#[tokio::test]
async fn login_cookie_secure_flag_respected() {
    let mut cfg = dev_config();
    cfg.cookie_secure = true;
    let state = match AppState::connect(&cfg).await { Ok(s)=>s, Err(_)=>{eprintln!("skip");return;} };
    let uname = make_user(&state, "pw1234").await;
    let app = build_router(state);
    let ok = post(app, "/api/login", &serde_json::json!({"username":uname,"password":"pw1234"}).to_string(), None).await;
    assert_eq!(ok.0, 200);
    let sc = ok.1.expect("set-cookie");
    assert!(sc.contains("Secure"), "secure=true면 Secure 속성 포함: {sc}");
    assert!(sc.contains("HttpOnly") && sc.contains("SameSite=Lax"));
}

#[tokio::test]
async fn rotate_token_returns_plaintext_once_and_authenticates_mcp() {
    let state = match test_state().await { Some(s)=>s, None=>{eprintln!("skip");return;} };
    let uname = make_user(&state, "pw1234").await;
    let app = build_router(state.clone());
    let login = post(app.clone(), "/api/login", &serde_json::json!({"username":uname,"password":"pw1234"}).to_string(), None).await;
    let cookie = extract_cookie(&login.1.unwrap());

    let r = post(app.clone(), "/api/token/rotate", "{}", Some(&cookie)).await;
    assert_eq!(r.0, 200);
    let v: serde_json::Value = serde_json::from_str(&r.2).unwrap();
    let token = v["token"].as_str().expect("token").to_string();
    assert!(token.len() >= 20);

    // 발급된 토큰이 실제 MCP 인증에 통한다
    let mcp_body = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}).to_string();
    let mut b = axum::http::Request::builder().method("POST").uri("/mcp")
        .header("content-type","application/json").header("authorization", format!("Bearer {token}"));
    let resp = app.oneshot(b.body(axum::body::Body::from(mcp_body)).unwrap()).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    // 쿠키 없이 rotate → 401
    let no = post(build_router(state), "/api/token/rotate", "{}", None).await;
    assert_eq!(no.0, 401);
}

#[tokio::test]
async fn upload_list_download_flow() {
    let state = match test_state().await { Some(s)=>s, None=>{eprintln!("skip");return;} };
    let uname = make_user(&state, "pw1234").await;
    let app = build_router(state.clone());
    let login = post(app.clone(), "/api/login", &serde_json::json!({"username":uname,"password":"pw1234"}).to_string(), None).await;
    let cookie = extract_cookie(&login.1.unwrap());

    // 업로드 URL 발급
    let up = post(app.clone(), "/api/upload-url", &serde_json::json!({"name":"계약서.hwp"}).to_string(), Some(&cookie)).await;
    assert_eq!(up.0, 200);
    let upv: serde_json::Value = serde_json::from_str(&up.2).unwrap();
    let handle = upv["handle"].as_str().unwrap().to_string();
    let upload_url = upv["upload_url"].as_str().unwrap().to_string();
    assert!(upload_url.contains("/upload/"), "앱 프록시 업로드 링크: {upload_url}");

    // 앱 프록시 업로드 라우트(/upload/{token})로 PUT (oneshot)
    let bytes = b"%HWP-FAKE-BYTES%\x00\x01";
    let up_path = &upload_url[upload_url.find("/upload/").unwrap()..];
    let put_resp = app.clone().oneshot(
        Request::builder().method("PUT").uri(up_path).body(Body::from(bytes.to_vec())).unwrap()
    ).await.unwrap();
    assert_eq!(put_resp.status().as_u16(), 200, "업로드 PUT 200");

    // 목록에 handle 존재
    let list = get(app.clone(), "/api/documents", Some(&cookie)).await;
    assert_eq!(list.0, 200);
    assert!(list.1.contains(&handle));

    // 다운로드 URL은 앱 프록시 링크(/download/{token})다. 경로를 뽑아 앱 라우트로 받는다.
    let dl = get(app.clone(), &format!("/api/download-url?handle={handle}"), Some(&cookie)).await;
    assert_eq!(dl.0, 200);
    let dlv: serde_json::Value = serde_json::from_str(&dl.1).unwrap();
    let download_url = dlv["download_url"].as_str().unwrap().to_string();
    assert!(download_url.contains("/download/"), "앱 프록시 링크: {download_url}");
    let path = &download_url[download_url.find("/download/").unwrap()..];
    // /download 라우트로 실제 바이트 수신(토큰이 인증, 쿠키 불필요)
    let resp = app.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let got = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    assert_eq!(&got[..], bytes);
}

#[tokio::test]
async fn admin_can_create_and_list_users_but_nonadmin_cannot() {
    let state = match test_state().await { Some(s)=>s, None=>{eprintln!("skip: dev 스택");return;} };
    let admin = make_admin(&state, "adminpw").await;
    let app = build_router(state.clone());

    // 관리자 로그인
    let login = post(app.clone(), "/api/login", &serde_json::json!({"username":admin,"password":"adminpw"}).to_string(), None).await;
    assert_eq!(login.0, 200);
    let cookie = extract_cookie(&login.1.unwrap());

    // /api/me 에 is_admin=true
    let me = get(app.clone(), "/api/me", Some(&cookie)).await;
    assert_eq!(me.0, 200);
    let mev: serde_json::Value = serde_json::from_str(&me.1).unwrap();
    assert_eq!(mev["is_admin"], serde_json::Value::Bool(true));

    // 관리자가 새 계정 생성 → 200
    let newu = format!("staff_{}", uuid::Uuid::new_v4());
    let create = post(app.clone(), "/api/admin/users", &serde_json::json!({"username":newu,"password":"pw1234","is_admin":false}).to_string(), Some(&cookie)).await;
    assert_eq!(create.0, 200, "생성 실패: {}", create.2);

    // 같은 아이디 재생성 → 409
    let dup = post(app.clone(), "/api/admin/users", &serde_json::json!({"username":newu,"password":"pw1234"}).to_string(), Some(&cookie)).await;
    assert_eq!(dup.0, 409);

    // 빈 값 → 400
    let empty = post(app.clone(), "/api/admin/users", &serde_json::json!({"username":"","password":""}).to_string(), Some(&cookie)).await;
    assert_eq!(empty.0, 400);

    // 목록에 새 계정 포함
    let list = get(app.clone(), "/api/admin/users", Some(&cookie)).await;
    assert_eq!(list.0, 200);
    assert!(list.1.contains(&newu), "목록에 새 계정: {}", list.1);

    // 방금 만든 비관리자로 로그인 후 관리자 API 접근 → 403
    let login2 = post(app.clone(), "/api/login", &serde_json::json!({"username":newu,"password":"pw1234"}).to_string(), None).await;
    assert_eq!(login2.0, 200);
    let cookie2 = extract_cookie(&login2.1.unwrap());
    let me2 = get(app.clone(), "/api/me", Some(&cookie2)).await;
    let me2v: serde_json::Value = serde_json::from_str(&me2.1).unwrap();
    assert_eq!(me2v["is_admin"], serde_json::Value::Bool(false));
    let forbidden = post(app.clone(), "/api/admin/users", &serde_json::json!({"username":"x","password":"y"}).to_string(), Some(&cookie2)).await;
    assert_eq!(forbidden.0, 403);
    let forbidden_list = get(app.clone(), "/api/admin/users", Some(&cookie2)).await;
    assert_eq!(forbidden_list.0, 403);

    // 쿠키 없이 → 401
    let anon = get(app, "/api/admin/users", None).await;
    assert_eq!(anon.0, 401);
}

#[tokio::test]
async fn index_serves_companion_page() {
    let state = match test_state().await { Some(s)=>s, None=>{eprintln!("skip");return;} };
    let app = build_router(state);
    let (code, body) = get(app, "/", None).await;
    assert_eq!(code, 200);
    // 페이지 식별 마커(제목)와 두 섹션 존재
    assert!(body.contains("rhwp"), "페이지 마커");
    assert!(body.contains("id=\"file-manager\""), "파일 매니저 섹션");
    assert!(body.contains("id=\"setup\""), "설정 섹션");
}
