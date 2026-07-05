//! PDF 렌더러 (Task #21)
//!
//! SVG 렌더러의 출력을 svg2pdf + pdf-writer로 PDF를 생성한다.
//! 단일/다중 페이지 모두 지원. 네이티브 전용 (WASM 미지원).

/// 폰트 데이터베이스를 초기화 (시스템 폰트 + 프로젝트 폰트 로드)
#[cfg(not(target_arch = "wasm32"))]
fn create_fontdb() -> usvg::fontdb::Database {
    let mut fontdb = usvg::fontdb::Database::new();
    fontdb.load_system_fonts();
    for dir in &["ttfs", "ttfs/windows", "ttfs/hwp"] {
        if std::path::Path::new(dir).exists() {
            fontdb.load_fonts_dir(dir);
        }
    }
    if std::path::Path::new("/mnt/c/Windows/Fonts").exists() {
        fontdb.load_fonts_dir("/mnt/c/Windows/Fonts");
    }
    // generic serif/sans 폴백: 서버(리눅스)엔 바탕/맑은고딕이 없어 한글 serif(함초롬바탕/명조)
    // 체인이 Latin 폰트로 폴백해 한글이 통째로 드롭됐다. 설치가 보장된 한글 폰트로 지정한다:
    // 서버는 fonts-nanum(NanumMyeongjo=serif, NanumGothic=sans) + ttfs Noto Sans KR.
    // 데스크톱(Windows)엔 이 이름이 없어도 폰트 체인의 명시 글꼴(바탕/맑은고딕)이 먼저 해석된다.
    fontdb.set_serif_family("NanumMyeongjo");
    fontdb.set_sans_serif_family("NanumGothic");
    fontdb.set_monospace_family("NanumGothicCoding");
    fontdb
}

/// SVG에서 없는 한글 폰트명에 fallback 추가
#[cfg(not(target_arch = "wasm32"))]
fn add_font_fallbacks(svg: &str) -> String {
    svg.replace(
        "font-family=\"휴먼명조\"",
        "font-family=\"휴먼명조, 바탕, serif\"",
    )
    .replace(
        "font-family=\"HCI Poppy\"",
        "font-family=\"HCI Poppy, 맑은 고딕, sans-serif\"",
    )
}

/// SVG를 PNG로 래스터화한다(resvg/tiny-skia, 순수 Rust). VLM(비전 모델) 입력용.
///
/// PDF 경로와 동일한 fontdb(시스템 fonts-nanum + ttfs)로 한글을 렌더하므로 폰트 정합이
/// 보장된다. `longest_edge`(px) 목표로 스케일(다운스케일 금지, 최소 1.0) — 텍스트 가독성 확보.
#[cfg(all(not(target_arch = "wasm32"), feature = "svg-raster"))]
pub fn svg_to_png(svg_content: &str, longest_edge: u32) -> Result<Vec<u8>, String> {
    let fontdb = create_fontdb();
    let mut options = usvg::Options::default();
    options.fontdb = std::sync::Arc::new(fontdb);
    // resvg 0.45 는 표 셀 clip-path(cell-clip-*)를 경계 안 텍스트에도 잘못 적용해
    // 여러 줄 셀 내용을 통째로 드롭한다(브라우저/mupdf 는 정상 렌더). 셀 클립만 제거한다 —
    // 텍스트는 이미 셀 폭에 맞춰 배치돼 있어 겹침 없이 온전히 그려진다.
    let svg_with_fallback = strip_cell_clips(&add_font_fallbacks(svg_content));
    let tree = usvg::Tree::from_str(&svg_with_fallback, &options)
        .map_err(|e| format!("SVG 파싱 실패: {e}"))?;
    let size = tree.size();
    let (bw, bh) = (size.width(), size.height());
    if bw <= 0.0 || bh <= 0.0 {
        return Err("페이지 크기가 0입니다".to_string());
    }
    let scale = (longest_edge as f32 / bw.max(bh)).max(1.0);
    let pw = (bw * scale).ceil() as u32;
    let ph = (bh * scale).ceil() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pw, ph)
        .ok_or_else(|| format!("Pixmap 생성 실패({pw}x{ph})"))?;
    pixmap.fill(resvg::tiny_skia::Color::WHITE); // 흰 배경(문서 분석용)
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|e| format!("PNG 인코딩 실패: {e}"))
}

/// SVG에서 표 셀 클립 속성(`clip-path="url(#cell-clip-...)"`)만 제거한다(정규식 의존 없음).
/// resvg 래스터 전용 보정 — PDF/브라우저 경로에는 적용하지 않는다.
#[cfg(all(not(target_arch = "wasm32"), feature = "svg-raster"))]
fn strip_cell_clips(svg: &str) -> String {
    const PAT: &str = "clip-path=\"url(#cell-clip";
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(i) = rest.find(PAT) {
        out.push_str(&rest[..i]);
        // 여는 `clip-path="` 뒤의 닫는 따옴표까지 스킵.
        let after = &rest[i + "clip-path=\"".len()..];
        match after.find('"') {
            Some(q) => rest = &after[q + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 단일 SVG를 PDF로 변환
#[cfg(not(target_arch = "wasm32"))]
pub fn svg_to_pdf(svg_content: &str) -> Result<Vec<u8>, String> {
    let fontdb = create_fontdb();
    let mut options = usvg::Options::default();
    options.fontdb = std::sync::Arc::new(fontdb);
    let svg_with_fallback = add_font_fallbacks(svg_content);
    let tree = usvg::Tree::from_str(&svg_with_fallback, &options)
        .map_err(|e| format!("SVG 파싱 실패: {}", e))?;
    let pdf = svg2pdf::to_pdf(
        &tree,
        svg2pdf::ConversionOptions::default(),
        svg2pdf::PageOptions::default(),
    )
    .map_err(|e| format!("PDF 변환 실패: {:?}", e))?;
    Ok(pdf)
}

/// 여러 SVG 페이지를 단일 다중 페이지 PDF로 생성
#[cfg(not(target_arch = "wasm32"))]
pub fn svgs_to_pdf(svg_pages: &[String]) -> Result<Vec<u8>, String> {
    if svg_pages.is_empty() {
        return Err("페이지가 없습니다".to_string());
    }
    if svg_pages.len() == 1 {
        return svg_to_pdf(&svg_pages[0]);
    }

    use pdf_writer::{Finish, Pdf, Ref};
    use std::collections::HashMap;

    let fontdb = create_fontdb();
    let mut options = usvg::Options::default();
    options.fontdb = std::sync::Arc::new(fontdb);

    let mut alloc = Ref::new(1);
    let catalog_ref = alloc.bump();
    let page_tree_ref = alloc.bump();

    // 각 페이지의 SVG를 파싱하여 chunk + page 정보 수집
    struct PageData {
        chunk: pdf_writer::Chunk,
        svg_ref: Ref,
        width: f32,
        height: f32,
    }

    let mut page_datas: Vec<PageData> = Vec::new();

    for svg in svg_pages {
        let svg_with_fallback = add_font_fallbacks(svg);
        let tree = usvg::Tree::from_str(&svg_with_fallback, &options)
            .map_err(|e| format!("SVG 파싱 실패: {}", e))?;

        let (chunk, svg_ref) = svg2pdf::to_chunk(&tree, svg2pdf::ConversionOptions::default())
            .map_err(|e| format!("SVG→chunk 변환 실패: {:?}", e))?;

        let dpi_ratio = 72.0 / 96.0; // 96 DPI → 72 pt
        let w = tree.size().width() * dpi_ratio;
        let h = tree.size().height() * dpi_ratio;

        page_datas.push(PageData {
            chunk,
            svg_ref,
            width: w,
            height: h,
        });
    }

    // 각 chunk를 재번호화하고 페이지 참조 수집
    let mut page_refs: Vec<Ref> = Vec::new();
    let mut renumbered_chunks: Vec<pdf_writer::Chunk> = Vec::new();
    let mut svg_refs_remapped: Vec<Ref> = Vec::new();

    for pd in &page_datas {
        let page_ref = alloc.bump();
        let content_ref = alloc.bump();
        page_refs.push(page_ref);

        // chunk 재번호화
        let mut map = HashMap::new();
        let renumbered = pd
            .chunk
            .renumber(|old| *map.entry(old).or_insert_with(|| alloc.bump()));

        let remapped_svg_ref = map.get(&pd.svg_ref).copied().unwrap_or(pd.svg_ref);
        svg_refs_remapped.push(remapped_svg_ref);
        renumbered_chunks.push(renumbered);
    }

    // PDF 생성
    let mut pdf = Pdf::new();
    pdf.catalog(catalog_ref).pages(page_tree_ref);
    pdf.pages(page_tree_ref)
        .count(page_refs.len() as i32)
        .kids(page_refs.iter().copied());

    // 각 페이지 생성
    let svg_name = pdf_writer::Name(b"S1");

    for (i, pd) in page_datas.iter().enumerate() {
        let page_ref = page_refs[i];
        let content_ref = alloc.bump();
        let svg_ref = svg_refs_remapped[i];

        let mut page = pdf.page(page_ref);
        page.media_box(pdf_writer::Rect::new(0.0, 0.0, pd.width, pd.height));
        page.parent(page_tree_ref);
        page.contents(content_ref);

        let mut resources = page.resources();
        resources.x_objects().pair(svg_name, svg_ref);
        resources.finish();
        page.finish();

        // 컨텐츠 스트림: SVG XObject를 페이지 크기에 맞게 배치
        let mut content = pdf_writer::Content::new();
        content.transform([pd.width, 0.0, 0.0, pd.height, 0.0, 0.0]);
        content.x_object(svg_name);

        pdf.stream(content_ref, &content.finish());
    }

    // 모든 chunk를 PDF에 추가
    for chunk in &renumbered_chunks {
        pdf.extend(chunk);
    }

    // 문서 정보
    let info_ref = alloc.bump();
    pdf.document_info(info_ref)
        .producer(pdf_writer::TextStr("rhwp"));

    Ok(pdf.finish())
}
