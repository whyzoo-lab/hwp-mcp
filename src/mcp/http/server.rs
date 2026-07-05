//! axum HTTP MCP 서버.
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use super::config::Config;
use super::db::Db;
use super::session::Sessions;
use super::store::Store;
use super::{authn, rpc, web};

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub store: Arc<Store>,
    pub sessions: Arc<Sessions>,
    pub web_sessions: Arc<web::WebSessions>,
    pub downloads: Arc<super::download::DownloadTokens>,
    pub uploads: Arc<super::download::UploadTokens>,
    pub cookie_secure: bool,
    pub public_base_url: String,
    /// `/mcp` 인증 요구 여부(false 면 인증 off = 단일 로컬 사용자 모드).
    pub auth_required: bool,
    /// 인증 off 모드에서 모든 요청이 귀속되는 기본 사용자. auth_required=true 면 None.
    pub default_user_id: Option<Uuid>,
}

impl AppState {
    pub async fn connect(cfg: &Config) -> Result<AppState, String> {
        let db = Db::connect(cfg).await?;
        let store = Store::connect(cfg).await?;
        // 인증 off 모드: 스키마 보장 후 기본 로컬 사용자를 확보해 문서 귀속을 준다.
        let default_user_id = if cfg.auth_required {
            None
        } else {
            db.migrate().await?; // 멱등
            let uid = db.ensure_local_user().await?;
            eprintln!("⚠ 인증 off 모드(RHWP_AUTH_REQUIRED=false): 모든 /mcp 요청이 단일 로컬 사용자로 처리됩니다. 공개 서버에서는 사용하지 마세요.");
            Some(uid)
        };
        Ok(AppState {
            db: Arc::new(db),
            store: Arc::new(store),
            sessions: Arc::new(Sessions::new()),
            web_sessions: Arc::new(web::WebSessions::new()),
            downloads: Arc::new(super::download::DownloadTokens::new()),
            uploads: Arc::new(super::download::UploadTokens::new()),
            cookie_secure: cfg.cookie_secure,
            public_base_url: cfg.public_base_url.clone(),
            auth_required: cfg.auth_required,
            default_user_id,
        })
    }
}

pub fn build_router(state: AppState) -> Router {
    // /mcp 요청 바디 상한을 명시적으로 지정(HWP/HTML 페이로드 고려, 16MB).
    // axum 기본값에 암묵적으로 의존하지 않고 의도를 코드로 드러낸다.
    // /healthz는 별도 라우트라 이 제한의 영향을 받지 않는다.
    const MCP_BODY_LIMIT: usize = 16 * 1024 * 1024;
    Router::new()
        .route("/", get(web::index))
        .route("/healthz", get(healthz))
        .route("/download/{token}", get(super::download::download))
        .route(
            "/upload/{token}",
            axum::routing::put(super::download::upload).layer(DefaultBodyLimit::max(MCP_BODY_LIMIT)),
        )
        .route(
            "/mcp",
            post(mcp_handler).layer(DefaultBodyLimit::max(MCP_BODY_LIMIT)),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(super::oauth::protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(super::oauth::authorization_server_metadata),
        )
        .route("/register", post(super::oauth::register))
        .route(
            "/authorize",
            get(super::oauth::authorize).post(super::oauth::authorize_consent),
        )
        .route("/token", post(super::oauth::token))
        .route("/api/login", post(web::login))
        .route("/api/logout", post(web::logout))
        .route("/api/me", get(web::me))
        .route(
            "/api/admin/users",
            get(web::admin_list_users).post(web::admin_create_user),
        )
        .route("/api/token/rotate", post(web::rotate_token))
        .route("/api/upload-url", post(web::upload_url))
        .route("/api/documents", get(web::list_docs))
        .route("/api/download-url", get(web::download_url))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn mcp_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    // 인증: 기본은 베어러/OAuth 필수. 인증 off 모드면 기본 로컬 사용자로 처리한다.
    let user_id = if !state.auth_required {
        state.default_user_id.expect("인증 off 모드엔 default_user_id 필수")
    } else {
        match authn::authenticate(&state.db, &headers).await {
            Ok(uid) => uid,
            Err(()) => {
                // 토큰 값/Authorization 헤더는 절대 로그에 남기지 않고 결과만 기록한다.
                eprintln!("인증 실패: 유효하지 않은 토큰");
                let mut h = HeaderMap::new();
                let wa = format!(
                    "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
                    state.public_base_url
                );
                h.insert("www-authenticate", wa.parse().unwrap());
                return (StatusCode::UNAUTHORIZED, h, Json(json!({"error":"인증 필요"})))
                    .into_response();
            }
        }
    };
    let sid = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("default")
        .to_string();

    let req: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return Json(rpc::error(
                Value::Null,
                rpc::PARSE_ERROR,
                &format!("parse error: {e}"),
            ))
            .into_response()
        }
    };
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let _params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    // Task 3~4에서 tools/call 디스패치를 super::rtools로 연결한다.
    let resp: Option<Value> = match method {
        "initialize" => id.clone().map(|id| {
            rpc::success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name":"rhwp-mcp-http","version": env!("CARGO_PKG_VERSION")},
                    "instructions": super::rtools::SERVER_INSTRUCTIONS
                }),
            )
        }),
        "notifications/initialized" | "notifications/cancelled" => None,
        "ping" => id.clone().map(|id| rpc::success(id, json!({}))),
        "tools/list" => id
            .clone()
            .map(|id| rpc::success(id, json!({"tools": super::rtools::tools_list_schema()}))),
        "tools/call" => {
            // 도구 실행을 별도 태스크로 격리한다. 코어 엔진이 패닉하더라도
            // 연결이 끊겨 502가 되는 대신, 태스크 경계에서 잡아 깨끗한
            // JSON-RPC 에러를 돌려주고 서버는 계속 살아있게 한다.
            let st = state.clone();
            let sidc = sid.clone();
            let paramsc = _params.clone();
            let idv = id.clone().unwrap_or(Value::Null);
            let idv_err = idv.clone();
            let joined = tokio::spawn(async move {
                super::rtools::handle_tools_call(&st, user_id, &sidc, idv, &paramsc).await
            })
            .await;
            Some(match joined {
                Ok(v) => v,
                Err(_) => {
                    // 패닉 내용/토큰은 로그에 남기지 않고 결과만 기록한다.
                    eprintln!("도구 실행 패닉으로 중단됨: tools/call");
                    rpc::error(
                        idv_err,
                        rpc::INTERNAL_ERROR,
                        "도구 실행 중 내부 오류가 발생했습니다(문서 내용이 원인일 수 있음)",
                    )
                }
            })
        }
        _ => id
            .clone()
            .map(|id| rpc::error(id, rpc::METHOD_NOT_FOUND, "method not found")),
    };
    match resp {
        Some(v) => Json(v).into_response(),
        None => StatusCode::ACCEPTED.into_response(), // 알림엔 바디 없음
    }
}

/// 프로세스 진입점: 설정으로 상태를 만들고 127.0.0.1:8300에 바인딩해 서빙.
pub async fn serve(cfg: Config) -> Result<(), String> {
    let state = AppState::connect(&cfg).await?;
    let app = build_router(state);
    let addr = std::env::var("RHWP_BIND").unwrap_or_else(|_| "127.0.0.1:8300".to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("바인딩 실패 {addr}: {e}"))?;
    eprintln!("rhwp-mcp-http 서빙: {addr}");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("서버 오류: {e}"))
}
