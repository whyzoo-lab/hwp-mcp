//! 회귀: HTML 인라인 스타일(b/i/span css)과 제목(h1/h2)이 export 후에도 보존되는지.
//! 과거 버그: css_to_char_shape_id/css_to_para_shape_id가 기본 모양 clone 시 raw_data를
//! 함께 복사 → 직렬화기가 raw_data를 우선해 수정 속성(굵게/크기/색/정렬)이 전부 유실.

fn export_bytes(html: &str) -> Vec<u8> {
    let mut doc = rhwp::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native().expect("blank");
    doc.paste_html_native(0, 0, 0, html).expect("paste");
    doc.export_hwp_with_adapter().expect("export")
}

#[test]
fn inline_styles_survive_export() {
    let html = r#"<p>일반 <b>굵게</b> <i>기울임</i> <span style="font-size:16pt;color:#ff0000">빨간큰글씨</span></p>"#;
    let bytes = export_bytes(html);
    let ir = rhwp::parser::parse_document(&bytes).expect("reparse");
    let shapes = &ir.doc_info.char_shapes;
    assert!(shapes.iter().any(|c| c.bold), "bold 보존");
    assert!(shapes.iter().any(|c| c.italic), "italic 보존");
    assert!(
        shapes.iter().any(|c| c.base_size == 1600),
        "16pt(1600) 보존: sizes={:?}",
        shapes.iter().map(|c| c.base_size).collect::<Vec<_>>()
    );
    assert!(
        shapes.iter().any(|c| c.text_color == 0x0000FF),
        "빨강(BGR 0x0000FF) 보존"
    );
}

#[test]
fn headings_are_separate_and_bold() {
    let html = "<h1>큰 제목</h1><h2>중간 제목</h2><p>본문</p>";
    let bytes = export_bytes(html);
    let ir = rhwp::parser::parse_document(&bytes).expect("reparse");
    let texts: Vec<&str> = ir.sections[0]
        .paragraphs
        .iter()
        .map(|p| p.text.as_str())
        .filter(|t| !t.is_empty())
        .collect();
    // 과거: "큰 제목중간 제목"으로 병합. 이제 별도 문단.
    assert!(
        texts.contains(&"큰 제목") && texts.contains(&"중간 제목"),
        "제목이 별도 문단이어야: {texts:?}"
    );
    let shapes = &ir.doc_info.char_shapes;
    assert!(
        shapes.iter().any(|c| c.bold && c.base_size == 1800),
        "h1=18pt 굵게 보존"
    );
    assert!(
        shapes.iter().any(|c| c.bold && c.base_size == 1500),
        "h2=15pt 굵게 보존"
    );
}

// --- MCP format_text / set_paragraph_format 핸들러가 만드는 props JSON이 코어에 먹히는지 ---

#[test]
fn format_text_props_apply_bold_color_size() {
    let mut doc = rhwp::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native().expect("blank");
    doc.paste_html_native(0, 0, 0, "<p>서식대상 텍스트</p>").expect("paste");
    // format_text 핸들러가 생성하는 props와 동일한 형태(bold/textColor/fontSize=pt*100)
    let props = r##"{"bold":true,"italic":true,"textColor":"#ff0000","fontSize":1600}"##;
    doc.apply_char_format_native(0, 0, 0, 4, props).expect("format"); // "서식대상"
    let bytes = export_bytes_of(&mut doc);
    let ir = rhwp::parser::parse_document(&bytes).expect("reparse");
    let s = &ir.doc_info.char_shapes;
    assert!(s.iter().any(|c| c.bold), "bold 적용");
    assert!(s.iter().any(|c| c.italic), "italic 적용");
    assert!(s.iter().any(|c| c.base_size == 1600), "16pt 적용");
    assert!(s.iter().any(|c| c.text_color == 0x0000FF), "빨강 적용");
}

#[test]
fn set_paragraph_format_props_apply_center() {
    let mut doc = rhwp::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native().expect("blank");
    doc.paste_html_native(0, 0, 0, "<p>문단</p>").expect("paste");
    doc.apply_para_format_native(0, 0, r#"{"alignment":"center"}"#).expect("para format");
    let bytes = export_bytes_of(&mut doc);
    let ir = rhwp::parser::parse_document(&bytes).expect("reparse");
    assert!(
        ir.doc_info.para_shapes.iter().any(|p| matches!(p.alignment, rhwp::model::style::Alignment::Center)),
        "가운데 정렬 적용"
    );
}

fn export_bytes_of(doc: &mut rhwp::wasm_api::HwpDocument) -> Vec<u8> {
    doc.export_hwp_with_adapter().expect("export")
}

#[test]
fn paragraph_alignment_survives_export() {
    let html = r#"<p style="text-align:center">가운데 문단</p>"#;
    let bytes = export_bytes(html);
    let ir = rhwp::parser::parse_document(&bytes).expect("reparse");
    let has_center = ir
        .doc_info
        .para_shapes
        .iter()
        .any(|p| matches!(p.alignment, rhwp::model::style::Alignment::Center));
    assert!(has_center, "가운데 정렬 보존");
}
