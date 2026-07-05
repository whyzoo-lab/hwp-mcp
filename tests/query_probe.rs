//! 회귀: 배치 A 쿼리 코어 메서드(표맵/라벨셀찾기/개요/스타일/셀분할).

fn doc_with(html: &str) -> rhwp::wasm_api::HwpDocument {
    let mut d = rhwp::wasm_api::HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, html).unwrap();
    d
}
fn table0(d: &rhwp::wasm_api::HwpDocument) -> (usize, usize, usize) {
    let t: serde_json::Value = serde_json::from_str(&d.list_tables_native()).unwrap();
    (
        t[0]["section"].as_u64().unwrap() as usize,
        t[0]["para"].as_u64().unwrap() as usize,
        t[0]["control"].as_u64().unwrap() as usize,
    )
}

#[test]
fn table_map_and_find_cell() {
    let d = doc_with("<table><tr><td>이름</td><td>홍길동</td></tr><tr><td>부서</td><td>개발</td></tr></table>");
    let map: serde_json::Value = serde_json::from_str(&d.get_table_map_native(0)).unwrap();
    assert_eq!(map["rows"], serde_json::json!(2));
    assert_eq!(map["cols"], serde_json::json!(2));
    let cells = map["cells"].as_array().unwrap();
    assert!(cells.iter().any(|c| c["text"] == serde_json::json!("이름")));
    assert!(cells.iter().any(|c| c["text"] == serde_json::json!("홍길동")));

    // 라벨 "이름" 오른쪽 = "홍길동"
    let r: serde_json::Value =
        serde_json::from_str(&d.find_cell_by_label_native(0, "이름", "right")).unwrap();
    assert_eq!(r["found"], serde_json::json!(true));
    assert_eq!(r["text"], serde_json::json!("홍길동"), "이름 오른쪽 셀");
    // 라벨 "부서" 오른쪽 = "개발"
    let r2: serde_json::Value =
        serde_json::from_str(&d.find_cell_by_label_native(0, "부서", "right")).unwrap();
    assert_eq!(r2["text"], serde_json::json!("개발"));
}

#[test]
fn split_cell_changes_count() {
    let mut d = doc_with("<table><tr><td>1</td><td>2</td></tr><tr><td>3</td><td>4</td></tr></table>");
    let (s, p, c) = table0(&d);
    let before: serde_json::Value = serde_json::from_str(&d.get_table_map_native(0)).unwrap();
    let n_before = before["cells"].as_array().unwrap().len();
    d.split_table_cell_into_native(s, p, c, 0, 0, 2, 2, true, false).unwrap();
    let after: serde_json::Value = serde_json::from_str(&d.get_table_map_native(0)).unwrap();
    let n_after = after["cells"].as_array().unwrap().len();
    assert!(n_after > n_before, "셀 분할로 셀 수 증가: {n_before}→{n_after}");
}

#[test]
fn outline_detects_headings() {
    let d = doc_with("<h1>큰 제목</h1><h2>중간 제목</h2><p>본문 문단</p>");
    let outline: serde_json::Value = serde_json::from_str(&d.get_document_outline_native()).unwrap();
    let items = outline.as_array().unwrap();
    assert!(items.iter().any(|i| i["text"] == serde_json::json!("큰 제목") && i["level"] == serde_json::json!(1)));
    assert!(items.iter().any(|i| i["text"] == serde_json::json!("중간 제목") && i["level"] == serde_json::json!(2)));
    // 본문은 제목이 아님
    assert!(!items.iter().any(|i| i["text"] == serde_json::json!("본문 문단")));
}

#[test]
fn list_styles_returns_array() {
    let d = doc_with("<p>본문</p>");
    let styles: serde_json::Value = serde_json::from_str(&d.list_styles_native()).unwrap();
    assert!(styles.is_array(), "스타일 목록은 배열");
}

#[test]
fn mail_merge_placeholder_replace() {
    let mut d = doc_with("<p>이름: {{name}}, 부서: {{dept}}</p>");
    d.replace_all_native("{{name}}", "홍길동", false).unwrap();
    d.replace_all_native("{{dept}}", "개발팀", false).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();
    let ir = rhwp::parser::parse_document(&bytes).unwrap();
    let text: String = ir.sections[0].paragraphs.iter().map(|p| p.text.clone()).collect();
    assert!(text.contains("홍길동") && text.contains("개발팀"), "치환 반영: {text}");
    assert!(!text.contains("{{"), "치환자 잔존 없음: {text}");
}
