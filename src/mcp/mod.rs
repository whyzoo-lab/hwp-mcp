//! rhwp MCP 서버 (stdio JSON-RPC). feature = "mcp"로 활성화.

#[cfg(feature = "mcp")]
pub mod protocol;
#[cfg(feature = "mcp")]
pub mod session;
#[cfg(feature = "mcp")]
pub mod tools;

#[cfg(feature = "mcp-http")]
pub mod http;

#[cfg(feature = "mcp")]
pub use protocol::serve_stdio;

#[cfg(feature = "mcp")]
use serde_json::{json, Value};
#[cfg(feature = "mcp")]
use session::{handle_tools_call, Session};

/// 지원 MCP 프로토콜 버전.
#[cfg(feature = "mcp")]
const PROTOCOL_VERSION: &str = "2024-11-05";

/// JSON-RPC 요청 1건을 처리한다. 알림(id 없음)이면 `None`.
#[cfg(feature = "mcp")]
pub fn dispatch(session: &mut Session, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => id.map(|id| {
            protocol::success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "rhwp-mcp", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
        }),
        "notifications/initialized" | "notifications/cancelled" => None,
        "tools/list" => id.map(|id| protocol::success(id, json!({"tools": tools::tools_list_schema()}))),
        "tools/call" => id.map(|id| handle_tools_call(session, id, &params)),
        "ping" => id.map(|id| protocol::success(id, json!({}))),
        _ => id.map(|id| protocol::error(id, protocol::METHOD_NOT_FOUND, "method not found")),
    }
}

/// `f`를 `catch_unwind`로 감싸 실행한다. 패닉 시 `id`가 있으면 INTERNAL_ERROR
/// 응답으로, 없으면(알림) `None`으로 변환한다. `dispatch_guarded`가 사용하는
/// 순수 헬퍼이며 단위 테스트에서 직접 검증한다.
#[cfg(feature = "mcp")]
pub fn guard<F>(id: Option<Value>, f: F) -> Option<Value>
where
    F: FnOnce() -> Option<Value> + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(f) {
        Ok(resp) => resp,
        // catch_unwind는 기본 패닉 훅을 통해 stderr에 메시지를 출력한 뒤 여기서 잡힌다.
        // stdout으로는 항상 정상적인 JSON-RPC 응답만 나가야 하므로 여기서 에러로 변환한다.
        Err(_) => id.map(|id| protocol::error(id, protocol::INTERNAL_ERROR, "요청 처리 중 내부 오류")),
    }
}

/// `dispatch`를 패닉으로부터 보호한다. stdio 루프가 도구 핸들러의 패닉으로
/// 죽지 않도록 `serve_stdio`가 `dispatch` 대신 이 함수를 호출한다.
#[cfg(feature = "mcp")]
pub fn dispatch_guarded(session: &mut Session, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    guard(id, std::panic::AssertUnwindSafe(|| dispatch(session, req)))
}
