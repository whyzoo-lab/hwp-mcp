//! 회귀: HWP에서 로드된(reloaded) 문서에 북마크를 추가할 때 섹션 raw_stream 캐시 때문에
//! 재직렬화에서 유실되던 버그. 원격 MCP는 매 호출마다 load→edit→save 하므로
//! add_bookmark는 항상 reloaded 문서에 적용된다 → 이 경로가 반드시 동작해야 한다.

fn reload(mut d: rhwp::wasm_api::HwpDocument) -> rhwp::wasm_api::HwpDocument {
    let b = d.export_hwp_with_adapter().unwrap();
    rhwp::wasm_api::HwpDocument::from_bytes(&b).unwrap()
}

#[test]
fn bookmark_on_reloaded_document_survives() {
    let mut d = rhwp::wasm_api::HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, "<p>본문 문단 하나입니다.</p>").unwrap();
    // MCP 흐름 모사: 문서를 한 번 저장→재로드한 뒤 북마크 추가.
    let mut d = reload(d);
    d.add_bookmark_native(0, 0, 0, "시작점").unwrap();
    // 다시 저장→재로드해도 북마크가 남아 있어야 한다.
    let d = reload(d);
    assert!(
        d.get_bookmarks_native().unwrap().contains("시작점"),
        "reloaded 문서에 추가한 북마크가 재직렬화에서 유실됨"
    );
}
