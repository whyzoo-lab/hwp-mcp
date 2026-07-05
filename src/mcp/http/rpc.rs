//! HTTP MCP용 JSON-RPC 2.0 헬퍼.
use serde_json::{json, Value};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const UNAUTHORIZED: i64 = -32001; // 애플리케이션 정의(인증)

pub fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
/// 도구 결과 JSON을 MCP content로 감싼다.
pub fn tool_text_result(value: &Value) -> Value {
    json!({"content":[{"type":"text","text": value.to_string()}],"isError":false})
}

/// PNG 바이트를 MCP 이미지 content(+캡션 텍스트)로 감싼다. LLM이 이미지로 직접 본다.
pub fn tool_image_result(png: &[u8], caption: &str) -> Value {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    json!({"content":[
        {"type":"image","data": b64, "mimeType":"image/png"},
        {"type":"text","text": caption},
    ],"isError":false})
}
