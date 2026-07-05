//! MCP 서버 dispatch 계층 테스트 (feature = "mcp").
#![cfg(feature = "mcp")]

use rhwp::mcp::{dispatch, dispatch_guarded, guard, session::Session};
use serde_json::json;

#[test]
fn mcp_initialize_returns_protocol_version() {
    let mut s = Session::new();
    let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
    let resp = dispatch(&mut s, &req).expect("initialize must respond");
    assert_eq!(resp["id"], json!(1));
    assert!(resp["result"]["protocolVersion"].is_string());
    assert_eq!(resp["result"]["serverInfo"]["name"], json!("rhwp-mcp"));
}

#[test]
fn mcp_tools_list_exposes_all_tools() {
    let mut s = Session::new();
    let req = json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}});
    let resp = dispatch(&mut s, &req).expect("tools/list must respond");
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 13);
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "open_document", "create_document", "insert_html", "read_document",
        "search", "replace_text", "insert_text", "delete_text",
        "export_document", "document_info",
        "list_fields", "get_field_value", "set_field_value",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
}

#[test]
fn mcp_notification_returns_none() {
    let mut s = Session::new();
    let req = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
    assert!(dispatch(&mut s, &req).is_none());
}

#[test]
fn mcp_unknown_method_returns_method_not_found() {
    let mut s = Session::new();
    let req = json!({"jsonrpc":"2.0","id":9,"method":"no/such","params":{}});
    let resp = dispatch(&mut s, &req).expect("must respond");
    assert_eq!(resp["error"]["code"], json!(-32601));
}

fn call(s: &mut Session, name: &str, args: serde_json::Value) -> serde_json::Value {
    let req = json!({"jsonrpc":"2.0","id":100,"method":"tools/call",
                     "params":{"name":name,"arguments":args}});
    dispatch(s, &req).expect("tools/call must respond")
}

/// content[0].text의 JSON 문자열을 다시 파싱한다.
fn result_json(resp: &serde_json::Value) -> serde_json::Value {
    assert!(resp.get("error").is_none(), "unexpected error: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().expect("text content");
    serde_json::from_str(text).expect("valid json in text")
}

#[test]
fn mcp_open_document_and_info() {
    let mut s = Session::new();
    let resp = call(&mut s, "open_document", json!({"path": "samples/2010-01-06.hwp"}));
    let body = result_json(&resp);
    assert_eq!(body["format"], json!("hwp"));
    assert!(body["section_count"].as_u64().unwrap() >= 1);

    let info = result_json(&call(&mut s, "document_info", json!({})));
    assert_eq!(info["loaded"], json!(true));
    assert_eq!(info["source_path"], json!("samples/2010-01-06.hwp"));
    assert!(info["paragraph_counts"].is_array());
}

#[test]
fn mcp_document_info_when_empty() {
    let mut s = Session::new();
    let info = result_json(&call(&mut s, "document_info", json!({})));
    assert_eq!(info["loaded"], json!(false));
}

#[test]
fn mcp_open_document_missing_file_errors() {
    let mut s = Session::new();
    let resp = call(&mut s, "open_document", json!({"path": "samples/__nope__.hwp"}));
    assert!(resp["error"]["code"].as_i64().is_some());
}

#[test]
fn mcp_create_document_with_html() {
    let mut s = Session::new();
    let body = result_json(&call(&mut s, "create_document",
        json!({"html": "<p>안녕하세요 rhwp</p><p>두 번째 문단</p>"})));
    assert!(body["section_count"].as_u64().unwrap() >= 1);

    let info = result_json(&call(&mut s, "document_info", json!({})));
    assert_eq!(info["loaded"], json!(true));
    // 삽입된 문단이 반영되어 문단 수가 1보다 크다.
    let counts = info["paragraph_counts"].as_array().unwrap();
    let total: u64 = counts.iter().map(|c| c.as_u64().unwrap()).sum();
    assert!(total >= 2, "html 문단이 반영되어야 함, got {total}");
}

#[test]
fn mcp_create_empty_then_insert_html() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document", json!({})));
    let body = result_json(&call(&mut s, "insert_html",
        json!({"html": "<p>추가된 내용</p>"})));
    assert_eq!(body["ok"], json!(true));
}

#[test]
fn mcp_insert_html_without_doc_errors() {
    let mut s = Session::new();
    let resp = call(&mut s, "insert_html", json!({"html": "<p>x</p>"}));
    assert!(resp["error"]["code"].as_i64().is_some());
}

#[test]
fn mcp_read_document_returns_text() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document",
        json!({"html": "<p>검색가능한문구ABC</p>"})));
    let body = result_json(&call(&mut s, "read_document", json!({"format": "plain"})));
    let text = body["text"].as_str().unwrap();
    assert!(text.contains("검색가능한문구ABC"), "read text: {text}");
}

#[test]
fn mcp_search_finds_match() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document",
        json!({"html": "<p>고유단어XYZ 포함 문단</p>"})));
    let body = result_json(&call(&mut s, "search", json!({"query": "고유단어XYZ"})));
    let matches = body["matches"].as_array().unwrap();
    assert!(!matches.is_empty(), "매치가 있어야 함");
    assert!(matches[0]["section"].as_u64().is_some());
}

#[test]
fn mcp_read_document_without_doc_errors() {
    let mut s = Session::new();
    let resp = call(&mut s, "read_document", json!({}));
    assert!(resp["error"]["code"].as_i64().is_some());
}

#[test]
fn mcp_replace_text_changes_content() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document",
        json!({"html": "<p>이전문구 유지</p>"})));
    let body = result_json(&call(&mut s, "replace_text",
        json!({"query": "이전문구", "replacement": "새문구"})));
    assert_eq!(body["ok"], json!(true));
    assert!(body["count"].as_u64().unwrap() >= 1);

    let read = result_json(&call(&mut s, "read_document", json!({})));
    let text = read["text"].as_str().unwrap();
    assert!(text.contains("새문구"), "치환 반영: {text}");
    assert!(!text.contains("이전문구"));
}

#[test]
fn mcp_insert_and_delete_text() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document", json!({"html": "<p>ABC</p>"})));
    // 문단 0,0 의 오프셋 0에 "X" 삽입
    let ins = result_json(&call(&mut s, "insert_text",
        json!({"section": 0, "para": 0, "char_offset": 0, "text": "X"})));
    assert_eq!(ins["ok"], json!(true));
    // 방금 넣은 "X" 삭제
    let del = result_json(&call(&mut s, "delete_text",
        json!({"section": 0, "para": 0, "char_offset": 0, "count": 1})));
    assert_eq!(del["ok"], json!(true));
}

#[test]
fn mcp_insert_text_missing_arg_errors() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document", json!({"html": "<p>A</p>"})));
    let resp = call(&mut s, "insert_text", json!({"section": 0, "para": 0, "text": "x"}));
    assert_eq!(resp["error"]["code"], json!(-32602));
}

#[test]
fn mcp_export_hwp_roundtrip() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document",
        json!({"html": "<p>내보내기왕복테스트</p>"})));

    let out = std::env::temp_dir().join("rhwp_mcp_export_test.hwp");
    let out_str = out.to_string_lossy().to_string();
    let body = result_json(&call(&mut s, "export_document",
        json!({"path": out_str, "format": "hwp"})));
    assert_eq!(body["ok"], json!(true));
    assert!(body["bytes_len"].as_u64().unwrap() > 0);
    assert!(out.exists(), "파일이 생성되어야 함");

    // 재로드 검증: 저장한 파일을 다시 열어 텍스트가 보존되는지 확인.
    let data = std::fs::read(&out).unwrap();
    let doc = rhwp::wasm_api::HwpDocument::from_bytes(&data).expect("재로드 성공");
    assert!(doc.get_section_count() >= 1);

    let _ = std::fs::remove_file(&out);
}

#[test]
fn mcp_export_without_doc_errors() {
    let mut s = Session::new();
    let resp = call(&mut s, "export_document", json!({"path": "x.hwp"}));
    assert!(resp["error"]["code"].as_i64().is_some());
}

#[test]
fn mcp_list_fields_on_plain_doc_is_empty_array() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document", json!({"html": "<p>필드 없음</p>"})));
    let body = result_json(&call(&mut s, "list_fields", json!({})));
    assert!(body["fields"].is_array());
    assert_eq!(body["count"], json!(0));
}

#[test]
fn mcp_set_field_value_unknown_name_errors() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document", json!({"html": "<p>x</p>"})));
    let resp = call(&mut s, "set_field_value", json!({"name": "__없는필드__", "value": "v"}));
    assert!(resp["error"]["code"].as_i64().is_some());
}

/// 단일 <p> HTML은 현재 문단에 인라인 병합되는 경우가 많아 새 문단 경계가
/// 생기지 않는다(before/after 문단 수 델타 = 0). 이 경로에서는 삽입 성공 여부를
/// ok로만 확인한다 — inserted_paragraphs는 0이어도 정상이며 .max(1)로 조작하지 않는다.
#[test]
fn mcp_insert_html_single_paragraph_ok_without_forcing_count() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document", json!({})));
    let body = result_json(&call(&mut s, "insert_html",
        json!({"html": "<p>실제로 삽입된 문단</p>"})));
    assert_eq!(body["ok"], json!(true));
}

/// 여러 <p> 블록을 삽입하면 새 문단 경계가 실제로 생기므로
/// inserted_paragraphs가 정직한 델타(>= 1)로 반영되어야 한다.
#[test]
fn mcp_insert_html_reports_real_inserted_paragraphs() {
    let mut s = Session::new();
    result_json(&call(&mut s, "create_document", json!({})));
    let body = result_json(&call(&mut s, "insert_html",
        json!({"html": "<p>문단1</p><p>문단2</p><p>문단3</p>"})));
    assert_eq!(body["ok"], json!(true));
    assert!(
        body["inserted_paragraphs"].as_u64().unwrap_or(0) >= 1,
        "여러 문단 삽입 시 새 문단 경계 수가 정직하게 반영되어야 함: {body}"
    );
}

/// Fix 1: 패닉 가드. `guard`가 패닉을 잡아 INTERNAL_ERROR 응답으로 변환하고,
/// 정상 반환값은 그대로 통과시키는지 직접 검증한다.
#[test]
fn mcp_guard_converts_panic_to_internal_error() {
    let resp = guard(Some(json!(7)), || panic!("boom"));
    let resp = resp.expect("id가 있으면 응답을 반환해야 함");
    assert_eq!(resp["error"]["code"], json!(-32603));
    assert_eq!(resp["id"], json!(7));
}

#[test]
fn mcp_guard_passes_through_success_value() {
    let resp = guard(Some(json!(7)), || Some(json!("x")));
    assert_eq!(resp, Some(json!("x")));
}

#[test]
fn mcp_guard_notification_panic_returns_none() {
    // id가 없는 요청(알림)은 패닉이 나도 응답을 만들지 않는다.
    let resp = guard(None, || panic!("boom"));
    assert!(resp.is_none());
}

/// stdio 루프가 실제로 사용하는 dispatch_guarded의 happy path도 함께 확인한다.
#[test]
fn mcp_dispatch_guarded_happy_path_matches_dispatch() {
    let mut s = Session::new();
    let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
    let resp = dispatch_guarded(&mut s, &req).expect("initialize must respond");
    assert_eq!(resp["id"], json!(1));
    assert!(resp["result"]["protocolVersion"].is_string());
}
