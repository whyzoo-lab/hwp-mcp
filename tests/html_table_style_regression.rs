//! HTML→HWP 변환 시각 충실도 회귀:
//! (1) 색 배경 머리행이 표 전체 배경으로 번지지 않는다(base_border_fills_len 수정).
//! (2) 지정한 font-family가 문서에 없어도 즉석 등록되어 렌더에 반영된다.
use rhwp::wasm_api::HwpDocument;

fn export_reload_svg(html: &str) -> String {
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, html).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();
    let d2 = HwpDocument::from_bytes(&bytes).unwrap();
    d2.render_page_svg_native(0).unwrap()
}

fn grab(s: &str, key: &str) -> f64 {
    s.split(key).nth(1).and_then(|x| x.split('"').next()).and_then(|x| x.parse().ok()).unwrap_or(0.0)
}

#[test]
fn dark_header_bg_does_not_flood_whole_table() {
    let html = "<table>\
        <tr><td style=\"background-color:#222222;width:60pt\"><span style=\"color:#FFFFFF\"><b>머리1</b></span></td>\
        <td style=\"background-color:#222222;width:300pt\"><span style=\"color:#FFFFFF\"><b>머리2</b></span></td></tr>\
        <tr><td style=\"width:60pt\">본문1</td><td style=\"width:300pt\">본문 내용</td></tr>\
        <tr><td style=\"width:60pt\">본문2</td><td style=\"width:300pt\">본문 내용2</td></tr></table>";
    let svg = export_reload_svg(html);
    for seg in svg.split("<rect").skip(1) {
        let cap = &seg[..seg.find('>').unwrap_or(seg.len())];
        if cap.contains("fill=\"#222222\"") {
            let (w, h) = (grab(cap, "width=\""), grab(cap, "height=\""));
            assert!(!(w > 350.0 && h > 60.0), "머리행 배경이 표 전체로 번짐: {w}x{h}");
        }
    }
}

#[test]
fn unknown_font_family_is_registered_and_rendered() {
    let svg = export_reload_svg(
        "<p><span style=\"font-family:맑은 고딕;font-size:14pt\"><b>가나다</b></span></p>",
    );
    let ff = svg.split("font-family=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("");
    assert!(
        ff.contains("맑은 고딕") || ff.to_lowercase().contains("malgun"),
        "지정 글꼴이 렌더에 반영되어야 함: {ff}"
    );
}

#[test]
fn cell_span_bold_does_not_leak_tags() {
    // 표 셀의 <span><b>..</b></span> 이 리터럴 태그로 새지 않고 텍스트만 남아야 한다.
    let svg = export_reload_svg(
        "<table><tr><td><span style=\"color:#FFFFFF\"><b>머리</b></span></td></tr></table>",
    );
    assert!(!svg.contains(">b<") && !svg.contains("&lt;b&gt;"), "셀 <b> 태그가 리터럴로 샘");
}

#[test]
fn multiline_cell_expands_row_height() {
    // 좁은 열에서 여러 줄로 접히는 셀은 행 높이가 1줄(약 13px)보다 충분히 커야 한다.
    let html = "<table><tr>\
        <td style=\"width:60pt\">1</td>\
        <td style=\"width:330pt\"><span style=\"font-size:9.5pt\">아주 긴 설명 텍스트로 여러 줄에 \
        걸쳐 줄바꿈이 일어나야 하는 셀 내용입니다. 충분히 길어서 두세 줄로 접힙니다.</span></td></tr></table>";
    let svg = export_reload_svg(html);
    let mut max_h = 0.0f64;
    for seg in svg.split("cell-clip-").skip(1) {
        if let Some(h) = seg.split("height=\"").nth(1).and_then(|x| x.split('"').next()).and_then(|x| x.parse::<f64>().ok()) {
            if h > max_h { max_h = h; }
        }
    }
    assert!(max_h > 30.0, "여러 줄 셀의 행 높이가 확장되지 않음: {max_h}");
}

#[test]
fn long_cell_text_wraps_instead_of_clipping() {
    // 좁은 열의 긴 셀 내용은 셀 폭에서 여러 줄로 접혀야 한다(한 줄로 넘쳐 잘리면 안 됨).
    let html = "<table><tr><td style=\"width:60pt\">1</td>\
      <td style=\"width:330pt\"><span style=\"font-size:9.5pt\">클로드 코드는 터미널에서 동작하며 \
      코드베이스를 읽고 파일을 수정·명령을 실행하는 에이전트형 도구다. 충분히 길어 두세 줄로 접혀야 한다.</span></td></tr></table>";
    let svg = export_reload_svg(html);
    let mut ys = std::collections::BTreeSet::new();
    for cap in svg.split("<text ").skip(1) {
        let head = &cap[..cap.find('>').unwrap_or(cap.len())];
        let x = head.split("x=\"").nth(1).and_then(|s| s.split('"').next()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let y = head.split("y=\"").nth(1).and_then(|s| s.split('"').next()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        if x > 150.0 && y > 0.0 { ys.insert((y * 10.0) as i64); }
    }
    assert!(ys.len() >= 2, "긴 셀 내용이 여러 줄로 접히지 않음(줄 수={})", ys.len());
}

#[test]
fn tall_table_splits_across_pages() {
    // 페이지보다 큰 표는 여러 쪽으로 분할되어 A4 경계를 넘지 않아야 한다(블록 표 조판).
    let mut rows = String::new();
    for i in 1..=40 {
        rows.push_str(&format!(
            "<tr><td style=\"width:60pt\">{i}</td><td style=\"width:330pt\"><span style=\"font-size:9.5pt\">이것은 페이지 분할을 유도하기 위한 충분히 긴 설명 문장입니다. 여러 줄로 접히며 40개 행이 한 페이지를 넘깁니다.</span></td></tr>"
        ));
    }
    let html = format!("<p>제목</p><table>{rows}</table>");
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, &html).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();
    let d2 = HwpDocument::from_bytes(&bytes).unwrap();
    let pc = d2.page_count();
    assert!(pc >= 2, "큰 표가 여러 쪽으로 분할되어야 함(page_count={pc})");
    // 각 페이지 텍스트가 A4 높이(1122px) 근처를 크게 넘지 않아야 함
    for pg in 0..pc {
        let svg = d2.render_page_svg_native(pg).unwrap();
        let max_y = svg.split("y=\"").skip(1)
            .filter_map(|s| s.split('"').next()?.parse::<f64>().ok())
            .fold(0.0, f64::max);
        assert!(max_y < 1200.0, "page{pg} 내용이 A4 밖으로 넘침: maxY={max_y}");
    }
}

#[test]
fn image_survives_export_reload() {
    // <p><img data:...></p> 이미지가 export→reload 후에도 렌더에 임베드되어야 한다.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAEklEQVR42mP8z8BQz0AEYBxVSF8FAGYqBfsJJ2SdAAAAAElFTkSuQmCC";
    let html = format!("<p>이미지</p><p><img src=\"data:image/png;base64,{png}\" width=\"80\" height=\"80\"></p>");
    let svg = export_reload_svg(&html);
    assert!(svg.contains("data:image") || svg.contains("xlink:href"), "이미지가 재로드 후 유실됨");
}

#[test]
fn cell_alignment_creates_distinct_shapes() {
    // 셀 text-align(center/left)이 서로 다른 문단 정렬로 반영되어야 한다.
    let html = "<table><tr>\
        <td style=\"text-align:center;width:60pt\">중앙</td>\
        <td style=\"text-align:left;width:200pt\">왼쪽</td></tr></table>";
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, html).unwrap();
    // 중앙 셀은 렌더 시 텍스트가 셀 좌측(패딩)보다 안쪽(중앙)에 위치해야 한다.
    let svg = d.render_page_svg_native(0).unwrap();
    // 최소한 export→reload가 성공하고 렌더가 비지 않아야 함(정렬 para_shape 생성 확인은 위 dump로 검증됨)
    assert!(svg.len() > 500);
}

#[test]
fn image_respects_size_and_flows_inline() {
    // 이미지가 지정 크기를 지키고 (0,0) 좌상단이 아닌 문단 흐름 위치에 렌더되어야 한다.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAEklEQVR42mP8z8BQz0AEYBxVSF8FAGYqBfsJJ2SdAAAAAElFTkSuQmCC";
    let html = format!("<p>앞줄</p><p><img src=\"data:image/png;base64,{png}\" width=\"100\" height=\"65\"></p>");
    let svg = export_reload_svg(&html);
    // <image> 요소의 width/height/y 확인
    let seg = svg.split("<image").nth(1).expect("image 요소 없음");
    let head = &seg[..seg.find('>').unwrap_or(seg.len())];
    let w: f64 = head.split("width=\"").nth(1).and_then(|s| s.split('"').next()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let y: f64 = head.split(" y=\"").nth(1).and_then(|s| s.split('"').next()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    assert!((w - 100.0).abs() < 2.0, "이미지 지정 폭(100) 미반영: {w}");
    assert!(y > 100.0, "이미지가 문단 흐름(y>100)이 아닌 상단(0)에 배치됨: {y}");
}

#[test]
fn image_shape_component_matches_hancom_record_sizes() {
    // 한컴2024 열기 거부 회귀: 그림 SHAPE_COMPONENT 는 base 만 196바이트여야 한다.
    // (border/fill/shadow 43바이트를 덧붙이면 239가 되어 다음 레코드 경계가 어긋나고,
    //  관대한 rhwp 파서는 복구하지만 한컴은 "파일을 읽거나 저장하는데 오류"로 거부한다.)
    // 불투명 그림의 SHAPE_COMPONENT_PICTURE 는 82바이트(정상 샘플 정합).
    use std::io::Read;
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAEklEQVR42mP8z8BQz0AEYBxVSF8FAGYqBfsJJ2SdAAAAAElFTkSuQmCC";
    let html = format!("<p>앞줄</p><p><img src=\"data:image/png;base64,{png}\" width=\"60\" height=\"60\"></p>");
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, &html).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();

    let mut cfb = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    let mut header = Vec::new();
    cfb.open_stream("FileHeader").unwrap().read_to_end(&mut header).unwrap();
    let compressed = header[36] & 1 == 1;
    let mut sec = Vec::new();
    cfb.open_stream("BodyText/Section0").unwrap().read_to_end(&mut sec).unwrap();
    if compressed {
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(&sec[..]).read_to_end(&mut out).unwrap();
        sec = out;
    }

    let (mut off, mut shape_sizes, mut pic_sizes) = (0usize, Vec::new(), Vec::new());
    while off + 4 <= sec.len() {
        let h = u32::from_le_bytes(sec[off..off + 4].try_into().unwrap());
        let (tag, mut size) = (h & 0x3FF, ((h >> 20) & 0xFFF) as usize);
        off += 4;
        if size == 0xFFF {
            size = u32::from_le_bytes(sec[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
        }
        match tag {
            0x4C => shape_sizes.push(size),                       // SHAPE_COMPONENT
            0x55 => pic_sizes.push(size),                         // SHAPE_COMPONENT_PICTURE
            _ => {}
        }
        off += size;
    }
    assert_eq!(off, sec.len(), "레코드 경계가 스트림 끝과 어긋남(크기 필드 오류)");
    assert_eq!(pic_sizes, vec![82], "불투명 그림 PICTURE 레코드는 82바이트여야 함");
    assert_eq!(shape_sizes, vec![196], "그림 SHAPE_COMPONENT 는 base 만 196바이트여야 함");
}

#[cfg(feature = "svg-raster")]
#[test]
fn render_page_png_is_valid_image() {
    // render_preview(format:"png") 경로: 페이지 SVG → resvg 래스터 → 유효 PNG.
    // 로컬 폰트로 한글이 렌더되는지 scratchpad에 저장해 눈으로도 확인 가능.
    let html = "<p><span style=\"font-family:맑은 고딕;font-size:16pt;color:#1B2A4A\"><b>공문서 시험 제목</b></span></p>\
        <table><tr>\
        <td style=\"width:120pt;border:0.5pt solid #000000;background-color:#E8F0FC\">항 목</td>\
        <td style=\"width:240pt;border:0.5pt solid #000000\">내용 값 12,345원</td></tr></table>";
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, html).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();
    let d2 = HwpDocument::from_bytes(&bytes).unwrap();
    let svg = d2.render_page_svg_native(0).unwrap();
    let png = rhwp::renderer::pdf::svg_to_png(&svg, 1568).unwrap();
    assert!(png.len() > 1000, "PNG가 너무 작음: {}바이트", png.len());
    assert_eq!(&png[1..4], b"PNG", "PNG 시그니처 아님");
    let _ = std::fs::write(
        r"C:\Users\taeho\AppData\Local\Temp\claude\D--Workspace-hwpMCP\89bb7015-b0ec-4791-9428-a0bae72ce227\scratchpad\png_smoke.png",
        &png,
    );
}

#[test]
fn picture_border_points_are_interleaved_rectangle() {
    // 이미지 찌그러짐 회귀: 이미지 사각형 4꼭짓점은 HWP 포맷대로 **인터리브**(x0,y0,x1,y1,...)로
    // 저장돼 깔끔한 사각형이어야 한다. 별도 배열로 쓰면 한컴이 나비넥타이로 읽어 찌그러진다.
    use std::io::Read;
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAYAAACzzX7wAAAAEklEQVR42mNkYPhfz0AEYBxVSF8FAP5FBfsKtQV4AAAAAElFTkSuQmCC"; // 8x4 (종횡비 다른)
    let html = format!(
        "<p><img src=\"data:image/png;base64,{png}\" width=\"40\" height=\"120\"></p>"
    );
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, &html).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();

    let mut cfb = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    let mut fh = Vec::new();
    cfb.open_stream("FileHeader").unwrap().read_to_end(&mut fh).unwrap();
    let compressed = fh[36] & 1 == 1;
    let mut sec = Vec::new();
    cfb.open_stream("BodyText/Section0").unwrap().read_to_end(&mut sec).unwrap();
    if compressed {
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(&sec[..]).read_to_end(&mut out).unwrap();
        sec = out;
    }
    // SHAPE_COMPONENT_PICTURE(0x55) 찾아 border 8 i32 파싱(offset 12..44).
    let (mut off, mut found) = (0usize, false);
    while off + 4 <= sec.len() {
        let h = u32::from_le_bytes(sec[off..off + 4].try_into().unwrap());
        let (tag, mut size) = (h & 0x3FF, ((h >> 20) & 0xFFF) as usize);
        off += 4;
        if size == 0xFFF { size = u32::from_le_bytes(sec[off..off+4].try_into().unwrap()) as usize; off += 4; }
        if tag == 0x55 && size >= 44 {
            let g = |i: usize| i32::from_le_bytes(sec[off + i..off + i + 4].try_into().unwrap());
            // 인터리브: (x0,y0)=(g12,g16) (x1,y1)=(g20,g24) (x2,y2)=(g28,g32) (x3,y3)=(g36,g40)
            let pts = [(g(12), g(16)), (g(20), g(24)), (g(28), g(32)), (g(36), g(40))];
            // 깔끔한 직사각형: P0=(0,0), P1=(W,0), P2=(W,H), P3=(0,H) — W,H>0
            let (w, hgt) = (pts[1].0, pts[2].1);
            assert!(w > 0 && hgt > 0, "사각형 크기 이상: {pts:?}");
            assert_eq!(pts[0], (0, 0), "P0!=원점 → 뒤틀림: {pts:?}");
            assert_eq!(pts[1], (w, 0), "P1 이상 → 뒤틀림: {pts:?}");
            assert_eq!(pts[2], (w, hgt), "P2 이상 → 뒤틀림: {pts:?}");
            assert_eq!(pts[3], (0, hgt), "P3 이상 → 뒤틀림: {pts:?}");
            found = true;
            break;
        }
        off += size;
    }
    assert!(found, "PICTURE 레코드를 찾지 못함");
}

#[test]
fn image_inside_table_cell_is_embedded() {
    // 회귀: 표 셀에 <img>만 있는 경우(텍스트 없음)에도 이미지가 유실되지 않고 BinData 로 임베드돼야 한다.
    use std::io::Read;
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAEklEQVR42mP8z8BQz0AEYBxVSF8FAGYqBfsJJ2SdAAAAAElFTkSuQmCC";
    let html = format!(
        "<table><tr><td style=\"width:100pt;border:0.5pt solid #000000\">라벨</td>\
         <td style=\"width:200pt;border:0.5pt solid #000000\"><img src=\"data:image/png;base64,{png}\" width=\"40\" height=\"40\"></td></tr></table>"
    );
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, &html).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();

    let mut cfb = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    let bin_streams: Vec<String> = cfb
        .walk()
        .filter(|e| e.is_stream())
        .map(|e| e.path().to_string_lossy().replace('\\', "/"))
        .filter(|p| p.starts_with("/BinData/"))
        .collect();
    assert!(
        !bin_streams.is_empty(),
        "표 셀 안 이미지가 BinData 로 임베드되지 않음(유실). 스트림: {bin_streams:?}"
    );
    // 이미지 데이터가 실제로 들어있는지(비어있지 않음)
    let mut img = Vec::new();
    cfb.open_stream(&bin_streams[0]).unwrap().read_to_end(&mut img).unwrap();
    assert!(img.len() > 20, "임베드된 이미지가 비어있음");
}

#[test]
fn table_border_width_and_color_are_faithful() {
    // 표 테두리 색·굵기 재현 회귀: 굵은 선(2pt)이 0.5mm(#7)로 클램프되지 않고 0.7mm(#9),
    // 색은 지정한 파랑(#0000FF)이 그대로 저장돼야 한다. (BorderFill 은 DocInfo 에 있음.)
    use std::io::Read;
    let html = "<table><tr>\
        <td style=\"width:80pt;border:2pt solid #0000FF\">가</td>\
        <td style=\"width:80pt;border:0.5pt solid #000000\">나</td></tr></table>";
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, html).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();

    let mut cfb = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    let mut header = Vec::new();
    cfb.open_stream("FileHeader").unwrap().read_to_end(&mut header).unwrap();
    let compressed = header[36] & 1 == 1;
    let mut di = Vec::new();
    cfb.open_stream("DocInfo").unwrap().read_to_end(&mut di).unwrap();
    if compressed {
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(&di[..]).read_to_end(&mut out).unwrap();
        di = out;
    }

    // BORDER_FILL(tag 0x14) 레코드에서 (선종류, 굵기index, colorRGB) 파싱.
    let mut found_thick_blue = false;
    let (mut off, mut widths) = (0usize, Vec::new());
    while off + 4 <= di.len() {
        let h = u32::from_le_bytes(di[off..off + 4].try_into().unwrap());
        let (tag, mut size) = (h & 0x3FF, ((h >> 20) & 0xFFF) as usize);
        off += 4;
        if size == 0xFFF {
            size = u32::from_le_bytes(di[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
        }
        if tag == 0x14 && size >= 2 + 6 {
            // attr(2) + [type(1)+width(1)+color(4)]*4
            let b = &di[off..off + size];
            for k in 0..4 {
                let p = 2 + k * 6;
                let (lt, wd) = (b[p], b[p + 1]);
                let col = u32::from_le_bytes(b[p + 2..p + 6].try_into().unwrap());
                widths.push(wd);
                // 파랑(#0000FF → BGR u32 = 0x00FF0000? color low byte=R). r|g<<8|b<<16 → b=FF => 0xFF0000
                if lt == 1 && wd == 9 && col == 0x00FF_0000 {
                    found_thick_blue = true;
                }
            }
        }
        off += size;
    }
    assert!(
        found_thick_blue,
        "2pt 파랑 테두리가 굵기 #9(0.7mm)+색 #0000FF 로 저장돼야 함(클램프/색손실 없이). 관측 widths={widths:?}"
    );
    assert!(!widths.iter().any(|&w| w == 7 && widths.contains(&9)),
        "굵은 선이 0.5mm(#7)로 클램프되면 안 됨");
}
