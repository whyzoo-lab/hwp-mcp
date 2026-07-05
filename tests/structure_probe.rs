//! 회귀: P2 구조 도구가 래핑하는 코어 명령(페이지설정/북마크/각주).

#[test]
fn page_setup_bookmark_footnote_core() {
    let mut doc = rhwp::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native().expect("blank");
    doc.paste_html_native(0, 0, 0, "<p>본문 문단 테스트입니다.</p>")
        .expect("paste");

    // 페이지 설정: 가로(landscape) + 왼쪽 여백 30mm (set_page_setup 핸들러와 동일 변환)
    let mm2hu = |mm: f64| (mm * 7200.0 / 25.4).round() as i64;
    let pj = format!("{{\"landscape\":true,\"marginLeft\":{}}}", mm2hu(30.0));
    doc.set_page_def_native(0, &pj).expect("page def");
    let pd: serde_json::Value =
        serde_json::from_str(&doc.get_page_def_native(0).expect("get pd")).expect("parse pd");
    assert_eq!(pd["landscape"], serde_json::json!(true), "가로 방향");
    assert_eq!(pd["marginLeft"], serde_json::json!(mm2hu(30.0)), "왼쪽 여백 30mm");

    // 북마크 추가 → 목록에 존재
    doc.add_bookmark_native(0, 0, 0, "mark1").expect("bookmark");
    let bms = doc.get_bookmarks_native().expect("get bookmarks");
    assert!(bms.contains("mark1"), "북마크 목록에 mark1: {bms}");

    // 각주 삽입 + 내용
    let raw = doc.insert_footnote_native(0, 0, 2).expect("footnote");
    let fnp: serde_json::Value = serde_json::from_str(&raw).expect("parse fn");
    assert_eq!(fnp["ok"], serde_json::json!(true));
    let ctrl = fnp["controlIdx"].as_u64().expect("controlIdx") as usize;
    doc.insert_text_in_footnote_native(0, 0, ctrl, 0, 0, "각주내용")
        .expect("footnote text");

    // export 정상(HWP CFB)
    let bytes = doc.export_hwp_with_adapter().expect("export");
    assert!(bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]), "HWP 산출");
}

#[test]
fn header_footer_survives_reload() {
    fn reload(mut d: rhwp::wasm_api::HwpDocument) -> rhwp::wasm_api::HwpDocument {
        let b = d.export_hwp_with_adapter().unwrap();
        rhwp::wasm_api::HwpDocument::from_bytes(&b).unwrap()
    }
    let mut d = rhwp::wasm_api::HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, "<p>본문 문단</p>").unwrap();
    // MCP 흐름 모사: 재로드된 문서에 머리말/꼬리말 추가
    let mut d = reload(d);
    let _ = d.create_header_footer_native(0, true, 0);
    d.insert_text_in_header_footer_native(0, true, 0, 0, 0, "문서제목")
        .unwrap();
    let _ = d.create_header_footer_native(0, false, 0);
    d.insert_field_in_hf_native(0, false, 0, 0, 0, 1).unwrap(); // 쪽번호
    // 재로드 후 머리말 텍스트 유지
    let d = reload(d);
    let hdr = d.get_header_footer_native(0, true, 0).unwrap();
    assert!(hdr.contains("문서제목"), "머리말이 재직렬화에서 유지: {hdr}");
}

#[test]
fn footer_text_plus_page_number_preserved() {
    fn reload(mut d: rhwp::wasm_api::HwpDocument) -> rhwp::wasm_api::HwpDocument {
        let b = d.export_hwp_with_adapter().unwrap();
        rhwp::wasm_api::HwpDocument::from_bytes(&b).unwrap()
    }
    fn footer_text(d: &rhwp::wasm_api::HwpDocument) -> usize {
        let j: serde_json::Value =
            serde_json::from_str(&d.get_header_footer_native(0, false, 0).unwrap()).unwrap();
        j["text"].as_str().unwrap_or("").chars().count()
    }
    let mut d = rhwp::wasm_api::HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, "<p>본문</p>").unwrap();
    let mut d = reload(d);
    // set_header_footer(footer="대외비")
    let _ = d.create_header_footer_native(0, false, 0);
    d.insert_text_in_header_footer_native(0, false, 0, 0, 0, "대외비").unwrap();
    let mut d = reload(d);
    // set_page_number: 기존 텍스트 끝에 필드 삽입(핸들러와 동일 로직)
    let _ = d.create_header_footer_native(0, false, 0);
    let end = footer_text(&d);
    d.insert_field_in_hf_native(0, false, 0, 0, end, 1).unwrap();
    let d = reload(d);
    let j: serde_json::Value =
        serde_json::from_str(&d.get_header_footer_native(0, false, 0).unwrap()).unwrap();
    assert!(
        j["text"].as_str().unwrap_or("").contains("대외비"),
        "쪽번호 삽입 후에도 꼬리말 텍스트 보존: {}",
        j["text"]
    );
}
