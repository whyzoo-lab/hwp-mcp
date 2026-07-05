// 서버 render_preview 0×0 근본원인 회귀 방지: paste_html에 표가 있을 때 export→reload
// 후에도 용지 크기(page_def)와 렌더 폭이 보존되어야 한다.
// (원인: html_import left_empty 분기가 구역 첫 문단의 SectionDef control을 파괴 → PAGE_DEF 유실)
use rhwp::wasm_api::HwpDocument;

fn roundtrip_reload_width(html: Option<&str>) -> (f64, f64) {
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    if let Some(h) = html {
        d.paste_html_native(0, 0, 0, h).unwrap();
    }
    let bytes = d.export_hwp_with_adapter().unwrap();
    let d2 = HwpDocument::from_bytes(&bytes).unwrap();
    let pd = d2.get_page_def_native(0).unwrap();
    let page_w: f64 = pd
        .split("\"width\":")
        .nth(1)
        .and_then(|s| s.split(&[',', '}'][..]).next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1.0);
    let svg = d2.render_page_svg_native(0).unwrap();
    let render_w = svg
        .split("width=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1.0);
    (page_w, render_w)
}

#[test]
fn table_html_preserves_pagedef_on_roundtrip() {
    let (pw, rw) = roundtrip_reload_width(Some(
        "<h1>제목</h1><p>본문</p><table><tr><td>문항</td><td>정답</td></tr><tr><td>1</td><td>2</td></tr></table>",
    ));
    assert!(pw > 50000.0, "표 포함 문서 reload page_def.width 유실: {pw}");
    assert!(rw > 100.0, "표 포함 문서 reload 렌더 폭 0 수렴: {rw}");
}

#[test]
fn plain_and_blank_html_still_roundtrip() {
    let (pw_b, rw_b) = roundtrip_reload_width(None);
    let (pw_t, rw_t) = roundtrip_reload_width(Some("<h1>제목</h1><p>본문</p>"));
    assert!(pw_b > 50000.0 && rw_b > 100.0, "blank 회귀: pw={pw_b} rw={rw_b}");
    assert!(pw_t > 50000.0 && rw_t > 100.0, "텍스트 회귀: pw={pw_t} rw={rw_t}");
}
