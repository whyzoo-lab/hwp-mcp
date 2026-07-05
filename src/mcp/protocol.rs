//! newline-delimited JSON-RPC 2.0 헬퍼 및 stdio 루프.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use super::{dispatch_guarded, session::Session};

/// 표준 JSON-RPC 에러 코드.
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// 성공 응답을 만든다.
pub fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// 에러 응답을 만든다.
pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// stdin에서 줄 단위 JSON-RPC를 읽어 처리하고 stdout으로 응답한다.
pub fn serve_stdio() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut session = Session::new();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_line(&mut out, &error(Value::Null, PARSE_ERROR, &format!("parse error: {e}")));
                continue;
            }
        };
        // dispatch_guarded: 도구 핸들러의 패닉이 stdio 루프 전체를 죽이지 않도록 감싼다.
        if let Some(resp) = dispatch_guarded(&mut session, &req) {
            write_line(&mut out, &resp);
        }
    }
}

fn write_line<W: Write>(out: &mut W, msg: &Value) {
    let _ = writeln!(out, "{}", msg);
    let _ = out.flush();
}
