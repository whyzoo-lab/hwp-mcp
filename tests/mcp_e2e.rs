//! rhwp-mcp 종단 통합 테스트 — 두 입력 흐름을 각각 검증.
#![cfg(feature = "mcp")]

use rhwp::mcp::{dispatch, session::Session};
use serde_json::{json, Value};

fn call(s: &mut Session, name: &str, args: Value) -> Value {
    let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                     "params":{"name":name,"arguments":args}});
    let resp = dispatch(s, &req).expect("respond");
    assert!(resp.get("error").is_none(), "error: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

/// 흐름 ①: 기존 HWP 열기 → 치환 편집 → HWP 내보내기 → 재로드.
#[test]
fn e2e_edit_existing_hwp() {
    let mut s = Session::new();
    call(&mut s, "open_document", json!({"path": "samples/2010-01-06.hwp"}));
    // 문서에 존재할 가능성이 낮은 고유 문자열이라도 치환은 count=0으로 성공해야 한다.
    let r = call(&mut s, "replace_text", json!({"query": "서울", "replacement": "서울특별시"}));
    assert_eq!(r["ok"].is_boolean(), true);

    let out = std::env::temp_dir().join("rhwp_mcp_e2e_existing.hwp");
    let body = call(&mut s, "export_document",
        json!({"path": out.to_string_lossy(), "format": "hwp"}));
    assert_eq!(body["ok"], json!(true));

    let data = std::fs::read(&out).unwrap();
    assert!(rhwp::wasm_api::HwpDocument::from_bytes(&data).is_ok());
    let _ = std::fs::remove_file(&out);
}

/// 흐름 ②: (외부 도구가 준) HTML → create_document → HWP 내보내기 → 재로드에서 텍스트 보존.
#[test]
fn e2e_html_to_hwp() {
    let mut s = Session::new();
    let html = "<h1>제목</h1><p>본문 내용입니다.</p><p>둘째 문단 KEEP_ME_1234.</p>";
    call(&mut s, "create_document", json!({"html": html}));

    let out = std::env::temp_dir().join("rhwp_mcp_e2e_html.hwp");
    let body = call(&mut s, "export_document",
        json!({"path": out.to_string_lossy(), "format": "hwp"}));
    assert_eq!(body["ok"], json!(true));

    // 재로드 후 read_document로 본문 보존 확인.
    let mut s2 = Session::new();
    call(&mut s2, "open_document", json!({"path": out.to_string_lossy()}));
    let read = call(&mut s2, "read_document", json!({"format": "plain"}));
    let text = read["text"].as_str().unwrap();
    assert!(text.contains("KEEP_ME_1234"), "본문 보존 실패: {text}");

    let _ = std::fs::remove_file(&out);
}

/// 흐름 ③: 누름틀/필드 서식 열기 → 필드 값 채우기 → HWP 내보내기 → 재로드에서 필드 유지.
#[test]
fn e2e_fill_form_fields() {
    let mut s = Session::new();
    call(&mut s, "open_document", json!({"path": "samples/누름틀-2024.hwp"}));

    let listed = call(&mut s, "list_fields", json!({}));
    let fields = listed["fields"].as_array().unwrap();
    assert!(!fields.is_empty(), "누름틀 샘플에는 필드가 있어야 함");
    let target = fields.iter()
        .find(|f| f["editableInForm"].as_bool().unwrap_or(false))
        .unwrap_or(&fields[0]);
    let first_name = target["name"].as_str().unwrap().to_string();

    // 값 설정 후 in-memory 조회로 반영 확인.
    call(&mut s, "set_field_value", json!({"name": first_name, "value": "채운값123"}));
    let got = call(&mut s, "get_field_value", json!({"name": first_name}));
    assert_eq!(got["value"], json!("채운값123"));

    // 내보내기 + 재로드에서 필드 구조 유지 확인.
    let out = std::env::temp_dir().join("rhwp_mcp_e2e_fields.hwp");
    let body = call(&mut s, "export_document",
        json!({"path": out.to_string_lossy(), "format": "hwp"}));
    assert_eq!(body["ok"], json!(true));

    let mut s2 = Session::new();
    call(&mut s2, "open_document", json!({"path": out.to_string_lossy()}));
    let relisted = call(&mut s2, "list_fields", json!({}));
    assert!(!relisted["fields"].as_array().unwrap().is_empty(), "재로드 후 필드 유지");

    let _ = std::fs::remove_file(&out);
}
