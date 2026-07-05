//! 회귀: paste_html_native 컨트롤 분기의 composed 삽입 순서 버그(rendering.rs:1979 패닉).
//! 원격 create_document와 동일한 코어 경로: create → paste_html → export.

fn create_paste_export(html: &str) -> Vec<u8> {
    let mut doc = rhwp::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native().expect("blank");
    doc.paste_html_native(0, 0, 0, html).expect("paste");
    doc.export_hwp_with_adapter().expect("export")
}

#[test]
fn h1_p_table_does_not_panic() {
    // 여러 블록 요소 + 다행 표(한글/원문자 셀) — 과거 rendering.rs:1979 패닉 케이스.
    let html = "<h1>원격 흐름 검증</h1><p>테스트</p><table><tr><td>문항</td><td>정답</td></tr><tr><td>1</td><td>②</td></tr></table>";
    let bytes = create_paste_export(html);
    assert!(bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]), "HWP(CFB) 산출");
}

#[test]
fn list_items_become_separate_paragraphs() {
    let mut doc = rhwp::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native().expect("blank");
    doc.paste_html_native(0, 0, 0, "<ul><li>항목1</li><li>항목2</li><li>항목3</li></ul>")
        .expect("paste");
    // 문단 텍스트 수집
    let sections = doc.get_section_count() as usize;
    let mut texts = Vec::new();
    for s in 0..sections {
        let paras = doc.get_paragraph_count_native(s).unwrap();
        for p in 0..paras {
            let len = doc.get_paragraph_length_native(s, p).unwrap();
            if len > 0 {
                texts.push(doc.get_text_range_native(s, p, 0, len).unwrap());
            }
        }
    }
    let joined = texts.join("|");
    assert!(
        joined.contains("항목1") && joined.contains("항목2") && joined.contains("항목3"),
        "항목 텍스트 보존: {texts:?}"
    );
    // 항목이 한 문단으로 붙지 않아야 함(과거: '항목1항목2항목3')
    assert!(
        !texts.iter().any(|t| t.contains("항목1항목2")),
        "리스트 항목이 별도 문단으로 분리되어야: {texts:?}"
    );
}

#[test]
fn multiblock_table_variants_do_not_panic() {
    let cases = [
        "<h1>제목</h1><p>본문</p><table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
        "<p>문단1</p><p>문단2</p><table><tr><td>1</td></tr><tr><td>2</td></tr><tr><td>3</td></tr></table>",
        "<h1>H</h1><p>P1</p><p>P2</p><table><tr><td>①</td><td>②</td><td>③</td></tr></table><p>뒤 문단</p>",
    ];
    for (i, html) in cases.iter().enumerate() {
        let bytes = create_paste_export(html);
        assert!(!bytes.is_empty(), "case {i} 비어있지 않음");
    }
}
