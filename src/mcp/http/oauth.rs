//! OAuth 2.1 (RS 겸 AS): 메타데이터/register/authorize/token. feature = "mcp-http".
use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::auth;
use super::server::AppState;
use super::web::cookie_value;

/// 코드/리프레시 토큰 해시(결정적 SHA-256). Task 4의 /token 조회도 동일 함수를 사용해야 한다.
/// (argon2는 매번 salt가 달라 code_hash로 재조회가 불가능하므로 여기서는 사용하지 않는다.)
pub fn sha256_b64url(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(d)
}

/// RFC 9728 Protected Resource Metadata.
pub async fn protected_resource_metadata(State(st): State<AppState>) -> impl IntoResponse {
    let base = &st.public_base_url;
    Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
    }))
}

/// RFC 8414 Authorization Server Metadata.
pub async fn authorization_server_metadata(State(st): State<AppState>) -> impl IntoResponse {
    let base = &st.public_base_url;
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code","refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"]
    }))
}

/// RFC 7591 Dynamic Client Registration. 공개 클라이언트(PKCE)만 지원(token_endpoint_auth_method=none).
pub async fn register(State(st): State<AppState>, body: String) -> impl IntoResponse {
    let v: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":"invalid_client_metadata"})))
                .into_response()
        }
    };
    let uris = v.get("redirect_uris").and_then(|x| x.as_array());
    let uris = match uris {
        Some(a) if !a.is_empty() => a.clone(),
        _ => {
            return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":"invalid_redirect_uri"})))
                .into_response()
        }
    };
    // 모두 문자열이고 https 또는 localhost인지 확인
    for u in &uris {
        let s = u.as_str().unwrap_or("");
        if !(s.starts_with("https://") || s.starts_with("http://localhost") || s.starts_with("http://127.0.0.1")) {
            return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":"invalid_redirect_uri"})))
                .into_response();
        }
    }
    let name = v.get("client_name").and_then(|x| x.as_str()).unwrap_or("");
    let uris_json = serde_json::to_string(&uris).unwrap();
    match st.db.create_oauth_client(&uris_json, name).await {
        Ok(cid) => (
            axum::http::StatusCode::CREATED,
            Json(json!({
                "client_id": cid, "redirect_uris": uris, "token_endpoint_auth_method": "none",
                "grant_types": ["authorization_code","refresh_token"], "response_types": ["code"]
            })),
        )
            .into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// 검증 통과한 authorize 파라미터.
struct AuthzParams {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    state: String,
    resource: Option<String>,
}

/// 응답타입/PKCE/코드챌린지 검증 + client_id 존재·redirect_uri 정확일치 확인.
/// GET(동의 렌더)와 POST(승인)에서 공용으로 쓴다.
async fn validate_authz(
    st: &AppState,
    q: &HashMap<String, String>,
) -> Result<AuthzParams, axum::response::Response> {
    let client_id = q.get("client_id").cloned().unwrap_or_default();
    let redirect_uri = q.get("redirect_uri").cloned().unwrap_or_default();
    let code_challenge = q.get("code_challenge").cloned().unwrap_or_default();
    let method = q.get("code_challenge_method").cloned().unwrap_or_default();
    let state = q.get("state").cloned().unwrap_or_default();
    // 빈 문자열 resource(폼 hidden 왕복)는 미지정(None)으로 정규화한다.
    let resource = q.get("resource").filter(|s| !s.is_empty()).cloned();
    if q.get("response_type").map(|s| s.as_str()) != Some("code")
        || method != "S256"
        || code_challenge.is_empty()
    {
        return Err((StatusCode::BAD_REQUEST, "invalid_request").into_response());
    }
    let redirects = match st.db.get_oauth_client_redirects(&client_id).await {
        Ok(Some(j)) => j,
        Ok(None) => return Err((StatusCode::BAD_REQUEST, "invalid_client").into_response()),
        Err(_) => return Err((StatusCode::INTERNAL_SERVER_ERROR, "server_error").into_response()),
    };
    let allowed: Vec<String> = serde_json::from_str(&redirects).unwrap_or_default();
    // redirect_uri는 등록된 목록과 정확일치해야 한다(open-redirect 방지, prefix/substring 불가).
    if !allowed.iter().any(|u| u == &redirect_uri) {
        return Err((StatusCode::BAD_REQUEST, "invalid_redirect_uri").into_response());
    }
    Ok(AuthzParams { client_id, redirect_uri, code_challenge, state, resource })
}

/// 세션 쿠키값에 묶인 동의 CSRF 토큰. 쿠키값은 HttpOnly 랜덤이라 공격자가 알 수 없고,
/// SameSite=Lax 로 cross-site POST 에는 쿠키 자체가 실리지 않으므로 위조 POST 를 이중 차단한다.
fn consent_csrf(cookie_val: &str) -> String {
    sha256_b64url(&format!("{cookie_val}|rhwp-oauth-consent-v1"))
}

/// GET /authorize: 미로그인 시 로그인 폼, 로그인 상태면 **동의 화면**을 렌더한다.
/// 여기서는 절대 code 를 발급하지 않는다 — 발급은 명시적 승인(POST /authorize)에서만.
/// (silent authorization + open DCR 로 인한 CSRF 인가코드 주입/계정탈취 차단.)
pub async fn authorize(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let p = match validate_authz(&st, &q).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    // 인증: 쿠키 세션 없으면 로그인 페이지(로그인 후 동일 authorize URL로 재요청 → 동의 화면).
    let cookie_val = match cookie_value(&headers) {
        Some(c) if st.web_sessions.user_of(&c).is_some() => c,
        _ => {
            let html = include_str!("authorize_login.html");
            return (StatusCode::OK, axum::response::Html(html)).into_response();
        }
    };
    let client_name = st.db.get_oauth_client_name(&p.client_id).await.ok().flatten().unwrap_or_default();
    let html = render_consent_page(&p, &client_name, &consent_csrf(&cookie_val));
    (StatusCode::OK, axum::response::Html(html)).into_response()
}

/// POST /authorize: 사용자가 동의 화면에서 승인/거부한 결과를 처리한다.
/// 세션 쿠키(로그인)와 CSRF 토큰을 검증한 뒤에만 code 를 발급한다.
pub async fn authorize_consent(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    let f = parse_form(&body);
    // 세션: cross-site POST 에는 Lax 쿠키가 실리지 않으므로 여기서 없으면 위조 요청이다.
    let cookie_val = match cookie_value(&headers) {
        Some(c) => c,
        None => return (StatusCode::UNAUTHORIZED, "login_required").into_response(),
    };
    let uid = match st.web_sessions.user_of(&cookie_val) {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "login_required").into_response(),
    };
    // CSRF: 세션 바인딩 토큰 일치 확인.
    if f.get("csrf").map(|s| s.as_str()) != Some(consent_csrf(&cookie_val).as_str()) {
        return (StatusCode::BAD_REQUEST, "invalid_csrf").into_response();
    }
    let p = match validate_authz(&st, &f).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    // 거부: redirect_uri 로 error=access_denied 반환(RFC 6749 §4.1.2.1).
    if f.get("action").map(|s| s.as_str()) == Some("deny") {
        let sep = if p.redirect_uri.contains('?') { '&' } else { '?' };
        let loc = format!(
            "{}{sep}error=access_denied&state={}",
            p.redirect_uri,
            pct_encode(&p.state)
        );
        let mut h = HeaderMap::new();
        h.insert("location", loc.parse().unwrap());
        return (StatusCode::FOUND, h).into_response();
    }
    // 승인 → code 발급.
    let code = auth::generate_token();
    // code_hash는 SHA-256(결정적)으로 저장. authorize/token 양쪽 동일 함수 사용.
    let code_hash = sha256_b64url(&code);
    if let Err(e) = st
        .db
        .insert_oauth_code(&code_hash, &p.client_id, uid, &p.redirect_uri, &p.code_challenge, p.resource.as_deref(), 600)
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    let sep = if p.redirect_uri.contains('?') { '&' } else { '?' };
    // code(base64url)는 안전하지만, state는 클라이언트 임의 문자열이므로 둘 다 percent-encode.
    let loc = format!("{}{sep}code={}&state={}", p.redirect_uri, pct_encode(&code), pct_encode(&p.state));
    let mut h = HeaderMap::new();
    h.insert("location", loc.parse().unwrap());
    (StatusCode::FOUND, h).into_response()
}

/// HTML 텍스트/속성 컨텍스트 이스케이프(동의 화면에 임의 문자열을 안전히 삽입).
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// 동의 화면: 요청 클라이언트/대상을 보여주고, 승인 시 same-site POST 로 code 를 받는다.
/// 모든 파라미터를 hidden 필드로 그대로 전달하되 값은 HTML 이스케이프한다.
fn render_consent_page(p: &AuthzParams, client_name: &str, csrf: &str) -> String {
    let name = if client_name.trim().is_empty() {
        "알 수 없는 애플리케이션".to_string()
    } else {
        html_escape(client_name)
    };
    let origin = html_escape(&p.redirect_uri);
    let hidden = |k: &str, v: &str| format!(r#"<input type="hidden" name="{k}" value="{}">"#, html_escape(v));
    let resource = p.resource.clone().unwrap_or_default();
    format!(
        r##"<!doctype html><html lang="ko"><head><meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>rhwp 접근 승인</title>
<style>
  body{{font-family:system-ui,-apple-system,"Segoe UI","Malgun Gothic",sans-serif;color:#1e1b3a;margin:0;min-height:100vh;display:grid;place-items:center;padding:1.5rem;background:linear-gradient(160deg,#eef2ff,#e0e7ff)}}
  .card{{width:100%;max-width:420px;background:#fff;border:1px solid #e5e7eb;border-radius:16px;padding:2rem 1.8rem;box-shadow:0 20px 45px -20px rgba(49,46,129,.45)}}
  .mark{{width:52px;height:52px;border-radius:14px;display:grid;place-items:center;background:linear-gradient(140deg,#4f46e5,#6366f1);color:#fff;font-weight:800;font-size:1.3rem;margin:0 auto .9rem}}
  h1{{font-size:1.15rem;margin:0 0 .3rem;text-align:center}}
  .sub{{color:#6b7280;font-size:.85rem;text-align:center;margin:0 0 1.2rem}}
  .who{{background:#f6f7ff;border:1px solid #e5e7eb;border-radius:10px;padding:.8rem 1rem;font-size:.9rem;margin-bottom:.6rem}}
  .who b{{color:#3730a3}} .who .u{{color:#6b7280;font-size:.78rem;word-break:break-all}}
  .warn{{color:#6b7280;font-size:.78rem;margin:.9rem 0 1.1rem;line-height:1.5}}
  .row{{display:flex;gap:.6rem}}
  button{{flex:1;font-size:.95rem;font-weight:600;padding:.7rem 1rem;border:0;border-radius:10px;cursor:pointer}}
  .approve{{background:linear-gradient(140deg,#4f46e5,#6366f1);color:#fff}}
  .deny{{background:#f1f1f5;color:#374151}}
</style></head><body>
<div class="card">
  <div class="mark">한</div>
  <h1>문서 접근 승인</h1>
  <p class="sub">아래 애플리케이션이 이 계정의 문서에 접근하려 합니다.</p>
  <div class="who"><b>{name}</b></div>
  <div class="who">전송 대상 <span class="u">{origin}</span></div>
  <p class="warn">승인하면 이 애플리케이션이 회원님 계정의 HWP 문서를 읽고 편집·생성할 수 있습니다. 요청한 앱이나 전송 대상 주소가 낯설면 <b>거부</b>하세요.</p>
  <form method="post" action="/authorize">
    {rt}{ci}{ru}{cc}{cm}{stt}{rsc}{csrf_f}
    <div class="row">
      <button class="deny" type="submit" name="action" value="deny">거부</button>
      <button class="approve" type="submit" name="action" value="approve">승인</button>
    </div>
  </form>
</div></body></html>"##,
        rt = hidden("response_type", "code"),
        ci = hidden("client_id", &p.client_id),
        ru = hidden("redirect_uri", &p.redirect_uri),
        cc = hidden("code_challenge", &p.code_challenge),
        cm = hidden("code_challenge_method", "S256"),
        stt = hidden("state", &p.state),
        rsc = hidden("resource", &resource),
        csrf_f = hidden("csrf", csrf),
    )
}

/// application/x-www-form-urlencoded 바디를 key→value 로 파싱한다.
fn parse_form(body: &str) -> HashMap<String, String> {
    let mut f = HashMap::new();
    for pair in body.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            f.insert(urldecode(k), urldecode(v));
        }
    }
    f
}

/// POST /token: 인가 코드(+PKCE) 또는 리프레시 토큰을 불투명 액세스 토큰으로 교환한다.
/// application/x-www-form-urlencoded 바디를 직접 파싱한다(신규 의존성 회피).
pub async fn token(State(st): State<AppState>, body: String) -> impl IntoResponse {
    let f = parse_form(&body);
    let grant = f.get("grant_type").map(|s| s.as_str()).unwrap_or("");
    match grant {
        "authorization_code" => {
            let code = f.get("code").cloned().unwrap_or_default();
            let redirect_uri = f.get("redirect_uri").cloned().unwrap_or_default();
            let verifier = f.get("code_verifier").cloned().unwrap_or_default();
            if code.is_empty() || verifier.is_empty() {
                return token_err(StatusCode::BAD_REQUEST, "invalid_request");
            }
            let code_hash = sha256_b64url(&code);
            let row = match st.db.take_valid_code(&code_hash).await {
                Ok(Some(r)) => r,
                Ok(None) => return token_err(StatusCode::BAD_REQUEST, "invalid_grant"),
                Err(_) => return token_err(StatusCode::INTERNAL_SERVER_ERROR, "server_error"),
            };
            let (uid, stored_redirect, challenge, client_id) = row;
            if stored_redirect != redirect_uri {
                return token_err(StatusCode::BAD_REQUEST, "invalid_grant");
            }
            // 코드는 발급 당시의 client_id에 귀속된다(RFC 6749 §4.1.3). 요청에 client_id가
            // 포함된 경우 저장된 값과 다르면 거부한다.
            if let Some(req_client_id) = f.get("client_id") {
                if !req_client_id.is_empty() && req_client_id != &client_id {
                    return token_err(StatusCode::BAD_REQUEST, "invalid_grant");
                }
            }
            // PKCE S256 검증
            if sha256_b64url(&verifier) != challenge {
                return token_err(StatusCode::BAD_REQUEST, "invalid_grant");
            }
            issue_tokens(&st, uid, &client_id).await
        }
        "refresh_token" => {
            let rt = f.get("refresh_token").cloned().unwrap_or_default();
            let rh = sha256_b64url(&rt);
            let (uid, client_id) = match st.db.consume_refresh(&rh).await {
                Ok(Some(x)) => x,
                Ok(None) => return token_err(StatusCode::BAD_REQUEST, "invalid_grant"),
                Err(_) => return token_err(StatusCode::INTERNAL_SERVER_ERROR, "server_error"),
            };
            issue_tokens(&st, uid, &client_id).await
        }
        _ => token_err(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
    }
}

fn token_err(code: StatusCode, err: &str) -> axum::response::Response {
    (code, Json(json!({"error": err}))).into_response()
}

async fn issue_tokens(st: &AppState, uid: uuid::Uuid, client_id: &str) -> axum::response::Response {
    // 액세스: 불투명 토큰 1h
    let access = auth::generate_token();
    let ah = match auth::hash_secret(&access) {
        Ok(h) => h,
        Err(_) => return token_err(StatusCode::INTERNAL_SERVER_ERROR, "server_error"),
    };
    if st.db.issue_token_expiring(uid, &ah, Some(3600)).await.is_err() {
        return token_err(StatusCode::INTERNAL_SERVER_ERROR, "server_error");
    }
    // 리프레시: 결정적 해시 저장, 30일
    let refresh = auth::generate_token();
    let rh = sha256_b64url(&refresh);
    if st.db.insert_refresh(&rh, uid, client_id, 60 * 60 * 24 * 30).await.is_err() {
        return token_err(StatusCode::INTERNAL_SERVER_ERROR, "server_error");
    }
    (
        StatusCode::OK,
        Json(json!({
            "access_token": access, "token_type": "Bearer", "expires_in": 3600, "refresh_token": refresh
        })),
    )
        .into_response()
}

/// 최소 percent-encoder(쿼리 값용). RFC 3986 unreserved(`A-Za-z0-9-_.~`) 외에는 모두 인코딩한다.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 최소 URL 디코더(form 값용).
fn urldecode(s: &str) -> String {
    let s = s.replace('+', " ");
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}
