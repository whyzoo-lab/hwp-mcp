//! Phase 5 OAuth 통합 테스트. feature = "mcp-http".
#![cfg(feature = "mcp-http")]
use rhwp::mcp::http::config::Config;
use rhwp::mcp::http::server::{build_router, AppState};
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn dev_config() -> Config {
    Config {
        database_url: "postgres://rhwp:rhwp@localhost:5433/rhwp".into(),
        s3_endpoint: "http://localhost:9002".into(), s3_region: "us-east-1".into(),
        s3_bucket: "rhwp-docs".into(), s3_access_key: "minioadmin".into(),
        s3_secret_key: "minioadmin".into(), s3_use_path_style: true,
        cookie_secure: false, public_base_url: "http://localhost:8300".into(),
        auth_required: true,
    }
}
async fn state() -> Option<AppState> { AppState::connect(&dev_config()).await.ok() }
async fn get(app: axum::Router, path:&str)->(u16,String,Option<String>){
    let r=app.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
    let code=r.status().as_u16(); let wa=r.headers().get("www-authenticate").and_then(|v|v.to_str().ok()).map(|s|s.to_string());
    let b=axum::body::to_bytes(r.into_body(),1<<20).await.unwrap();
    (code,String::from_utf8_lossy(&b).to_string(),wa)
}

#[tokio::test]
async fn metadata_documents_served() {
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    let app=build_router(st);
    let (c1,b1,_)=get(app.clone(),"/.well-known/oauth-protected-resource").await;
    assert_eq!(c1,200);
    let v1:serde_json::Value=serde_json::from_str(&b1).unwrap();
    assert!(v1["authorization_servers"].as_array().unwrap().iter().any(|s|s.as_str()==Some("http://localhost:8300")));
    let (c2,b2,_)=get(app,"/.well-known/oauth-authorization-server").await;
    assert_eq!(c2,200);
    let v2:serde_json::Value=serde_json::from_str(&b2).unwrap();
    assert_eq!(v2["authorization_endpoint"],serde_json::json!("http://localhost:8300/authorize"));
    assert_eq!(v2["token_endpoint"],serde_json::json!("http://localhost:8300/token"));
    assert_eq!(v2["registration_endpoint"],serde_json::json!("http://localhost:8300/register"));
    assert!(v2["code_challenge_methods_supported"].as_array().unwrap().iter().any(|s|s.as_str()==Some("S256")));
}

#[tokio::test]
async fn mcp_401_has_www_authenticate() {
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    let app=build_router(st);
    // 토큰 없이 /mcp POST → 401 + WWW-Authenticate(resource_metadata)
    let r=app.oneshot(Request::builder().method("POST").uri("/mcp").header("content-type","application/json").body(Body::from("{}")).unwrap()).await.unwrap();
    assert_eq!(r.status().as_u16(),401);
    let wa=r.headers().get("www-authenticate").expect("WWW-Authenticate").to_str().unwrap().to_string();
    assert!(wa.contains("resource_metadata="), "헤더에 resource_metadata: {wa}");
    assert!(wa.contains("/.well-known/oauth-protected-resource"));
}

async fn post_json(app: axum::Router, path:&str, body:&str)->(u16,String){
    let r=app.oneshot(Request::builder().method("POST").uri(path).header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let code=r.status().as_u16(); let b=axum::body::to_bytes(r.into_body(),1<<20).await.unwrap();
    (code,String::from_utf8_lossy(&b).to_string())
}
#[tokio::test]
async fn dcr_register_returns_client_id() {
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    st.db.migrate().await.unwrap();
    let app=build_router(st);
    let (c,b)=post_json(app,"/register",r#"{"redirect_uris":["https://claude.ai/api/mcp/auth_callback"],"client_name":"claude"}"#).await;
    assert_eq!(c,201);
    let v:serde_json::Value=serde_json::from_str(&b).unwrap();
    assert!(v["client_id"].as_str().unwrap().len()>=10);
    assert_eq!(v["redirect_uris"][0],serde_json::json!("https://claude.ai/api/mcp/auth_callback"));
}

#[tokio::test]
async fn auth_off_mode_allows_mcp_without_token() {
    // RHWP_AUTH_REQUIRED=false 모드: 토큰 없이 /mcp 가 동작(단일 로컬 사용자로 귀속)해야 한다.
    let mut cfg = dev_config();
    cfg.auth_required = false;
    let st = match AppState::connect(&cfg).await { Ok(s)=>s, Err(_)=>{eprintln!("skip");return;} };
    assert!(st.default_user_id.is_some(), "인증 off 모드는 기본 로컬 사용자를 확보해야 함");
    let app = build_router(st);
    // 토큰 없는 initialize → 200 (401 아님)
    let r=app.clone().oneshot(Request::builder().method("POST").uri("/mcp").header("content-type","application/json").body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)).unwrap()).await.unwrap();
    assert_eq!(r.status().as_u16(),200,"인증 off 모드는 토큰 없이 200이어야 함");
    // 문서 생성도 토큰 없이 동작
    let cr=app.oneshot(Request::builder().method("POST").uri("/mcp").header("content-type","application/json").body(Body::from(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_document","arguments":{"name":"authoff"}}}"#)).unwrap()).await.unwrap();
    assert_eq!(cr.status().as_u16(),200,"인증 off 모드는 토큰 없이 도구 호출 가능해야 함");
}

#[tokio::test]
async fn auth_on_mode_still_rejects_missing_token() {
    // 기본(auth_required=true) 모드는 여전히 토큰 없는 /mcp 를 401로 거부해야 한다(회귀 방지).
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    let app=build_router(st);
    let r=app.oneshot(Request::builder().method("POST").uri("/mcp").header("content-type","application/json").body(Body::from("{}")).unwrap()).await.unwrap();
    assert_eq!(r.status().as_u16(),401);
}

async fn login_cookie(app: axum::Router, st:&AppState)->String{
    st.db.migrate().await.unwrap();
    let uname=format!("oauth_{}",uuid::Uuid::new_v4());
    let hash=rhwp::mcp::http::auth::hash_secret("pw").unwrap();
    st.db.create_user(&uname,&hash,false).await.unwrap();
    let r=app.oneshot(Request::builder().method("POST").uri("/api/login").header("content-type","application/json").body(Body::from(format!("{{\"username\":\"{uname}\",\"password\":\"pw\"}}"))).unwrap()).await.unwrap();
    r.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_string()
}
#[tokio::test]
async fn authorize_get_renders_consent_not_code_when_logged_in() {
    // 보안 회귀: 로그인된 세션 쿠키만으로 GET /authorize 가 code 를 발급하면 안 된다
    // (silent authorization + open DCR → CSRF 인가코드 주입/계정탈취). 동의 화면(200)만.
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    st.db.migrate().await.unwrap();
    let redir="https://claude.ai/api/mcp/auth_callback";
    let cid=st.db.create_oauth_client(&serde_json::to_string(&[redir]).unwrap(),"claude").await.unwrap();
    let app=build_router(st.clone());
    let cookie=login_cookie(app.clone(),&st).await;
    let url=format!("/authorize?response_type=code&client_id={cid}&redirect_uri={}&code_challenge=abc123&code_challenge_method=S256&state=xyz&resource=http%3A%2F%2Flocalhost%3A8300%2Fmcp",urlencoding_min(redir));
    let r=app.clone().oneshot(Request::builder().uri(&url).header("cookie",&cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status().as_u16(),200,"GET은 동의 화면(200)");
    assert!(r.headers().get("location").is_none(),"GET에 code redirect가 있으면 안 됨");
    let b=axum::body::to_bytes(r.into_body(),1<<20).await.unwrap();
    let html=String::from_utf8_lossy(&b);
    assert!(html.contains("문서 접근 승인") && html.contains("name=\"csrf\""),"동의 화면 폼이어야 함");
    // 명시적 POST 승인만이 302 code 를 낸다.
    let loc=approve_authorize(&app,&cookie,&url).await;
    assert!(loc.starts_with(redir) && loc.contains("code=") && loc.contains("state=xyz"),"redirect: {loc}");
}

#[tokio::test]
async fn authorize_post_rejects_without_session_or_csrf() {
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    st.db.migrate().await.unwrap();
    let redir="https://claude.ai/api/mcp/auth_callback";
    let cid=st.db.create_oauth_client(&serde_json::to_string(&[redir]).unwrap(),"claude").await.unwrap();
    let app=build_router(st.clone());
    let cookie=login_cookie(app.clone(),&st).await;
    let body=format!("action=approve&response_type=code&client_id={cid}&redirect_uri={}&code_challenge=abc123&code_challenge_method=S256&state=xyz&csrf=bogus",urlencoding_min(redir));
    // 세션 쿠키 없음 → 401 (cross-site POST 는 Lax 로 쿠키 미전송, 이 경로에 해당)
    let no_cookie=app.clone().oneshot(Request::builder().method("POST").uri("/authorize").header("content-type","application/x-www-form-urlencoded").body(Body::from(body.clone())).unwrap()).await.unwrap();
    assert_eq!(no_cookie.status().as_u16(),401,"세션 없는 POST는 거부");
    // 세션은 있으나 CSRF 토큰 위조 → 400
    let bad_csrf=app.oneshot(Request::builder().method("POST").uri("/authorize").header("cookie",&cookie).header("content-type","application/x-www-form-urlencoded").body(Body::from(body)).unwrap()).await.unwrap();
    assert_eq!(bad_csrf.status().as_u16(),400,"CSRF 불일치 POST는 거부");
}
#[tokio::test]
async fn authorize_bad_redirect_uri_400() {
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    st.db.migrate().await.unwrap();
    let cid=st.db.create_oauth_client(&serde_json::to_string(&["https://claude.ai/api/mcp/auth_callback"]).unwrap(),"claude").await.unwrap();
    let app=build_router(st.clone());
    let cookie=login_cookie(app.clone(),&st).await;
    let url=format!("/authorize?response_type=code&client_id={cid}&redirect_uri=https%3A%2F%2Fevil.example%2Fcb&code_challenge=abc&code_challenge_method=S256&state=z");
    let r=app.oneshot(Request::builder().uri(&url).header("cookie",&cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status().as_u16(),400);
}
// 간단 URL 인코더(테스트용)
fn urlencoding_min(s:&str)->String{ s.replace(':',"%3A").replace('/',"%2F") }

/// 동의 화면(GET /authorize) HTML에서 hidden 필드 value를 추출한다.
fn hidden_val(html:&str, name:&str)->String{
    let key=format!("name=\"{name}\" value=\"");
    let start=html.find(&key).unwrap_or_else(||panic!("hidden {name} 없음"))+key.len();
    html[start..].split('"').next().unwrap().to_string()
}

/// 로그인 상태로 authorize 2단계(GET 동의→POST 승인)를 수행해 302 Location을 반환한다.
async fn approve_authorize(app:&axum::Router, cookie:&str, url:&str)->String{
    // 1) GET /authorize → 동의 화면 HTML (code 미발급)
    let g=app.clone().oneshot(Request::builder().uri(url).header("cookie",cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(g.status().as_u16(),200,"GET authorize는 동의 화면(200)이어야 함");
    let gb=axum::body::to_bytes(g.into_body(),1<<20).await.unwrap();
    let html=String::from_utf8_lossy(&gb).to_string();
    let csrf=hidden_val(&html,"csrf");
    // 2) POST /authorize (승인) → 302 code
    let form=format!("action=approve&response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}&csrf={}",
        hidden_val(&html,"client_id"),
        urlencoding_min(&hidden_val(&html,"redirect_uri")),
        hidden_val(&html,"code_challenge"),
        hidden_val(&html,"state"),
        urlencoding_min(&csrf));
    let r=app.clone().oneshot(Request::builder().method("POST").uri("/authorize").header("cookie",cookie).header("content-type","application/x-www-form-urlencoded").body(Body::from(form)).unwrap()).await.unwrap();
    assert_eq!(r.status().as_u16(),302,"POST 승인은 302여야 함");
    r.headers().get("location").unwrap().to_str().unwrap().to_string()
}

#[tokio::test]
async fn full_oauth_flow_issues_token_that_authenticates_mcp() {
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    st.db.migrate().await.unwrap();
    let redir="https://claude.ai/api/mcp/auth_callback";
    let cid=st.db.create_oauth_client(&serde_json::to_string(&[redir]).unwrap(),"claude").await.unwrap();
    let app=build_router(st.clone());
    let cookie=login_cookie(app.clone(),&st).await;
    // PKCE
    let verifier="verifier-0123456789-0123456789-0123456789";
    let challenge={use sha2::{Digest,Sha256};use base64::Engine;base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))};
    // authorize (동의→승인) → code
    let url=format!("/authorize?response_type=code&client_id={cid}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state=s",urlencoding_min(redir));
    let loc=approve_authorize(&app,&cookie,&url).await;
    let code=loc.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
    // token(정상 verifier) → access
    let form=format!("grant_type=authorization_code&code={code}&redirect_uri={}&code_verifier={verifier}&client_id={cid}",urlencoding_min(redir));
    let tr=app.clone().oneshot(Request::builder().method("POST").uri("/token").header("content-type","application/x-www-form-urlencoded").body(Body::from(form)).unwrap()).await.unwrap();
    assert_eq!(tr.status().as_u16(),200);
    let tb=axum::body::to_bytes(tr.into_body(),1<<20).await.unwrap();
    let tv:serde_json::Value=serde_json::from_slice(&tb).unwrap();
    let access=tv["access_token"].as_str().unwrap().to_string();
    assert_eq!(tv["token_type"],serde_json::json!("Bearer"));
    // access 토큰으로 /mcp initialize
    let mr=app.oneshot(Request::builder().method("POST").uri("/mcp").header("authorization",format!("Bearer {access}")).header("content-type","application/json").body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)).unwrap()).await.unwrap();
    assert_eq!(mr.status().as_u16(),200);
}
#[tokio::test]
async fn token_pkce_mismatch_400() {
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    st.db.migrate().await.unwrap();
    let redir="https://claude.ai/api/mcp/auth_callback";
    let cid=st.db.create_oauth_client(&serde_json::to_string(&[redir]).unwrap(),"claude").await.unwrap();
    let app=build_router(st.clone());
    let cookie=login_cookie(app.clone(),&st).await;
    let challenge="t4gY...definitely-not-matching";
    let url=format!("/authorize?response_type=code&client_id={cid}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state=s",urlencoding_min(redir));
    let loc=approve_authorize(&app,&cookie,&url).await;
    let code=loc.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
    let form=format!("grant_type=authorization_code&code={code}&redirect_uri={}&code_verifier=WRONG&client_id={cid}",urlencoding_min(redir));
    let tr=app.oneshot(Request::builder().method("POST").uri("/token").header("content-type","application/x-www-form-urlencoded").body(Body::from(form)).unwrap()).await.unwrap();
    assert_eq!(tr.status().as_u16(),400);
}

#[tokio::test]
async fn token_wrong_client_id_400() {
    let st=match state().await{Some(s)=>s,None=>{eprintln!("skip");return;}};
    st.db.migrate().await.unwrap();
    let redir="https://claude.ai/api/mcp/auth_callback";
    let cid=st.db.create_oauth_client(&serde_json::to_string(&[redir]).unwrap(),"claude").await.unwrap();
    // 코드가 귀속되지 않은, 별도로 등록된 다른 client_id
    let other_cid=st.db.create_oauth_client(&serde_json::to_string(&[redir]).unwrap(),"other").await.unwrap();
    let app=build_router(st.clone());
    let cookie=login_cookie(app.clone(),&st).await;
    let verifier="verifier-0123456789-0123456789-0123456789";
    let challenge={use sha2::{Digest,Sha256};use base64::Engine;base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))};
    let url=format!("/authorize?response_type=code&client_id={cid}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state=s",urlencoding_min(redir));
    let loc=approve_authorize(&app,&cookie,&url).await;
    let code=loc.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
    // /token에 코드 발급 시와 다른 client_id를 제시 → 400 invalid_grant
    let form=format!("grant_type=authorization_code&code={code}&redirect_uri={}&code_verifier={verifier}&client_id={other_cid}",urlencoding_min(redir));
    let tr=app.oneshot(Request::builder().method("POST").uri("/token").header("content-type","application/x-www-form-urlencoded").body(Body::from(form)).unwrap()).await.unwrap();
    assert_eq!(tr.status().as_u16(),400);
    let tb=axum::body::to_bytes(tr.into_body(),1<<20).await.unwrap();
    let tv:serde_json::Value=serde_json::from_slice(&tb).unwrap();
    assert_eq!(tv["error"],serde_json::json!("invalid_grant"));
}
