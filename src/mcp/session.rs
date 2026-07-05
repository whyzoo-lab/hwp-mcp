//! MCP 세션 상태와 도구 호출 디스패치.

use serde_json::{json, Value};

use super::protocol::{error, success, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};

/// 활성 문서 포맷.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DocFormat {
    Hwp,
    Hwpx,
}

/// 프로세스 수명 동안 유지되는 세션. 활성 문서 1개.
pub struct Session {
    /// 활성 문서 (HWP/HWPX).
    pub doc: Option<crate::wasm_api::HwpDocument>,
    pub source_path: Option<String>,
    pub format: Option<DocFormat>,
}

impl Session {
    pub fn new() -> Self {
        Session { doc: None, source_path: None, format: None }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// 활성 문서가 있으면 가변 참조를 반환한다.
    pub fn require_doc(&mut self) -> Result<&mut crate::wasm_api::HwpDocument, (i64, String)> {
        self.doc
            .as_mut()
            .ok_or_else(|| (INVALID_PARAMS, "열린 문서가 없습니다. 먼저 open_document 또는 create_document를 호출하세요.".to_string()))
    }
}

/// 파일 확장자로 포맷을 추정한다.
pub fn detect_format(path: &str) -> DocFormat {
    if path.to_ascii_lowercase().ends_with(".hwpx") {
        DocFormat::Hwpx
    } else {
        DocFormat::Hwp
    }
}

impl DocFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            DocFormat::Hwp => "hwp",
            DocFormat::Hwpx => "hwpx",
        }
    }
}

/// tools/call 처리. `id`는 상위(dispatch)에서 붙인다.
pub fn handle_tools_call(session: &mut Session, id: Value, params: &Value) -> Value {
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return error(id, INVALID_PARAMS, "tools/call: 'name' 필드 누락"),
    };
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    // 도구 핸들러는 성공 시 JSON Value, 실패 시 (code, message)를 반환.
    let result: Result<Value, (i64, String)> = match name {
        "open_document" => super::tools::open_document(session, &args),
        "create_document" => super::tools::create_document(session, &args),
        "insert_html" => super::tools::insert_html(session, &args),
        "read_document" => super::tools::read_document(session, &args),
        "search" => super::tools::search(session, &args),
        "replace_text" => super::tools::replace_text(session, &args),
        "insert_text" => super::tools::insert_text(session, &args),
        "delete_text" => super::tools::delete_text(session, &args),
        "export_document" => super::tools::export_document(session, &args),
        "document_info" => super::tools::document_info(session, &args),
        "list_fields" => super::tools::list_fields(session, &args),
        "get_field_value" => super::tools::get_field_value(session, &args),
        "set_field_value" => super::tools::set_field_value(session, &args),
        _ => Err((METHOD_NOT_FOUND, format!("알 수 없는 도구: {name}"))),
    };

    match result {
        Ok(value) => success(id, tool_text_result(&value)),
        Err((code, message)) => error(id, code, &message),
    }
}

/// 도구 결과 JSON을 MCP content 형식으로 감싼다.
pub fn tool_text_result(value: &Value) -> Value {
    json!({
        "content": [{"type": "text", "text": value.to_string()}],
        "isError": false
    })
}
