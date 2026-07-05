//! 회귀: P1b 표 편집 코어 메서드(list_tables_native, set_cell_text_native).

#[test]
fn list_tables_and_set_cell_text() {
    let mut doc = rhwp::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native().expect("blank");
    doc.paste_html_native(
        0,
        0,
        0,
        "<table><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></table>",
    )
    .expect("paste");

    // list_tables_native: 2행×2열 표 1개
    let json = doc.list_tables_native();
    let tables: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(tables.as_array().map(|a| a.len()), Some(1), "표 1개: {json}");
    let t = &tables[0];
    let sec = t["section"].as_u64().unwrap() as usize;
    let para = t["para"].as_u64().unwrap() as usize;
    let ctrl = t["control"].as_u64().unwrap() as usize;
    assert_eq!(t["rows"], serde_json::json!(2));
    assert_eq!(t["cols"], serde_json::json!(2));

    // set_cell_text_native: (1,1) 셀 "D" → "변경됨"
    doc.set_cell_text_native(sec, para, ctrl, 1, 1, "변경됨")
        .expect("set cell");

    // export → 재파싱하여 셀 (1,1) 텍스트 확인
    let bytes = doc.export_hwp_with_adapter().expect("export");
    let ir = rhwp::parser::parse_document(&bytes).expect("reparse");
    let mut cell_11: Option<String> = None;
    for p in &ir.sections[0].paragraphs {
        for c in &p.controls {
            if let rhwp::model::control::Control::Table(tb) = c {
                for cell in &tb.cells {
                    if cell.row == 1 && cell.col == 1 {
                        cell_11 = Some(cell.paragraphs.iter().map(|pp| pp.text.clone()).collect());
                    }
                }
            }
        }
    }
    assert_eq!(cell_11.as_deref(), Some("변경됨"), "셀(1,1) 텍스트 교체 반영");
}

#[test]
fn insert_delete_row_column_changes_dimensions() {
    let mut doc = rhwp::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native().expect("blank");
    doc.paste_html_native(
        0,
        0,
        0,
        "<table><tr><td>1</td><td>2</td></tr><tr><td>3</td><td>4</td></tr></table>",
    )
    .expect("paste");
    let t: serde_json::Value = serde_json::from_str(&doc.list_tables_native()).unwrap();
    let (sec, para, ctrl) = (
        t[0]["section"].as_u64().unwrap() as usize,
        t[0]["para"].as_u64().unwrap() as usize,
        t[0]["control"].as_u64().unwrap() as usize,
    );
    // 행 삽입(2→3), 열 삽입(2→3)
    doc.insert_table_row_native(sec, para, ctrl, 0, true).expect("row+");
    doc.insert_table_column_native(sec, para, ctrl, 0, true).expect("col+");
    let t2: serde_json::Value = serde_json::from_str(&doc.list_tables_native()).unwrap();
    assert_eq!(t2[0]["rows"], serde_json::json!(3), "행 3");
    assert_eq!(t2[0]["cols"], serde_json::json!(3), "열 3");
    // 행 삭제(3→2)
    doc.delete_table_row_native(sec, para, ctrl, 0).expect("row-");
    let t3: serde_json::Value = serde_json::from_str(&doc.list_tables_native()).unwrap();
    assert_eq!(t3[0]["rows"], serde_json::json!(2), "행 2");
}
