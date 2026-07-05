//! HTML 표 파싱 + BorderFill 생성 + 이미지 파싱 관련 native 메서드

use super::helpers::*;
use crate::document_core::DocumentCore;
use crate::error::HwpError;
use crate::model::control::Control;
use crate::model::paragraph::Paragraph;
use crate::model::shape::common_obj_offsets;
use crate::renderer::style_resolver::resolve_styles;

impl DocumentCore {
    pub(crate) fn parse_table_html(&mut self, paragraphs: &mut Vec<Paragraph>, table_html: &str) {
        use crate::model::control::Control;
        use crate::model::table::{Cell, Table, TablePageBreak};

        // --- 1. HTML 파싱: 행/셀 구조 추출 ---
        let table_lower = table_html.to_lowercase();

        struct ParsedCell {
            col_span: u16,
            row_span: u16,
            width_pt: f64,
            height_pt: f64,
            padding_pt: [f64; 4],          // left, right, top, bottom
            border_widths_pt: [f64; 4],    // left, right, top, bottom
            border_colors: [u32; 4],       // BGR
            border_styles: [u8; 4],        // 0=none, 1=solid, 2=dashed, 3=dotted, 4=double
            background_color: Option<u32>, // BGR
            content_html: String,
            is_header: bool,
            vertical_align: u8, // 0=top, 1=center, 2=bottom
            text_align: Option<String>, // 셀 문단 가로 정렬(CSS 값: center/right/justify/left)
        }

        let mut parsed_rows: Vec<Vec<ParsedCell>> = Vec::new();

        let mut pos = 0;
        while let Some(tr_start) = table_lower[pos..].find("<tr") {
            let tr_abs = pos + tr_start;
            let tr_end = find_closing_tag(table_html, tr_abs, "tr");
            let tr_inner = &table_html[tr_abs..tr_end.min(table_html.len())];
            let tr_inner_lower = tr_inner.to_lowercase();

            let mut row_cells: Vec<ParsedCell> = Vec::new();

            // <td>와 <th>를 출현 순서대로 처리
            let mut td_pos = 0;
            loop {
                let td_match = tr_inner_lower[td_pos..].find("<td");
                let th_match = tr_inner_lower[td_pos..].find("<th");

                let (tag_offset, is_th) = match (td_match, th_match) {
                    (Some(a), Some(b)) => {
                        if a <= b {
                            (a, false)
                        } else {
                            (b, true)
                        }
                    }
                    (Some(a), None) => (a, false),
                    (None, Some(b)) => (b, true),
                    (None, None) => break,
                };

                let cell_abs = td_pos + tag_offset;
                let tag_name = if is_th { "th" } else { "td" };

                // <td ...> 태그에서 속성 추출
                if let Some(gt) = tr_inner[cell_abs..].find('>') {
                    let tag_str = &tr_inner[cell_abs..cell_abs + gt + 1];

                    // colspan / rowspan 파싱
                    let col_span = parse_html_attr_u16(tag_str, "colspan").unwrap_or(1).max(1);
                    let row_span = parse_html_attr_u16(tag_str, "rowspan").unwrap_or(1).max(1);

                    // 인라인 style 파싱
                    let css = parse_inline_style(tag_str);
                    let css_lower = css.to_lowercase();

                    // 크기 파싱
                    let width_pt = parse_css_dimension_pt(&css_lower, "width");
                    let height_pt = parse_css_dimension_pt(&css_lower, "height");

                    // 패딩 파싱
                    let padding_pt = parse_css_padding_pt(&css_lower);

                    // 테두리 파싱 (left, right, top, bottom)
                    let mut border_widths_pt = [0.0f64; 4];
                    let mut border_colors = [0u32; 4]; // black
                    let mut border_styles = [1u8; 4]; // solid default

                    // 축약형 border 먼저
                    if let Some(bval) = parse_css_value(&css_lower, "border") {
                        let (w, c, s) = parse_css_border_shorthand(&bval);
                        for i in 0..4 {
                            border_widths_pt[i] = w;
                            border_colors[i] = c;
                            border_styles[i] = s;
                        }
                    }
                    // 개별 방향 오버라이드
                    let sides = ["border-left", "border-right", "border-top", "border-bottom"];
                    for (i, side) in sides.iter().enumerate() {
                        if let Some(bval) = parse_css_value(&css_lower, side) {
                            let (w, c, s) = parse_css_border_shorthand(&bval);
                            border_widths_pt[i] = w;
                            border_colors[i] = c;
                            border_styles[i] = s;
                        }
                    }

                    // 배경색
                    let background_color = parse_css_value(&css_lower, "background-color")
                        .or_else(|| parse_css_value(&css_lower, "background"))
                        .and_then(|v| css_color_to_hwp_bgr(&v));

                    // 수직 정렬 (0=미지정, 1=center, 2=bottom, 3=명시적 top)
                    let vertical_align =
                        match parse_css_value(&css_lower, "vertical-align").as_deref() {
                            Some("middle") | Some("center") => 1u8,
                            Some("bottom") => 2u8,
                            Some("top") => 3u8, // 명시적 top
                            _ => 0u8,           // 미지정 → Center (HWP 기본)
                        };

                    // 가로 정렬 (text-align) — 셀 문단 정렬. CSS 값 그대로 보관했다가
                    // 사용처에서 css_to_para_shape_id 로 넘긴다(enum 왕복 변환 제거).
                    let text_align = parse_css_value(&css_lower, "text-align")
                        .filter(|v| matches!(v.as_str(), "center" | "right" | "justify" | "left"));

                    // 셀 내용 HTML 추출
                    let content_start = cell_abs + gt + 1;
                    let close_tag = format!("</{}>", tag_name);
                    let content_end =
                        if let Some(close) = tr_inner_lower[content_start..].find(&close_tag) {
                            content_start + close
                        } else {
                            tr_inner.len()
                        };
                    let content_html = tr_inner[content_start..content_end].to_string();

                    row_cells.push(ParsedCell {
                        col_span,
                        row_span,
                        width_pt,
                        height_pt,
                        padding_pt,
                        border_widths_pt,
                        border_colors,
                        border_styles,
                        background_color,
                        content_html,
                        is_header: is_th,
                        vertical_align,
                        text_align,
                    });

                    td_pos = content_end + close_tag.len();
                } else {
                    break;
                }
            }

            if !row_cells.is_empty() {
                parsed_rows.push(row_cells);
            }
            pos = tr_end;
        }

        if parsed_rows.is_empty() {
            return;
        }

        // --- 2. 그리드 정규화: 실제 col 인덱스 계산 ---
        let row_count = parsed_rows.len() as u16;
        // colspan 합산으로 최대 열 수 추정
        let mut max_cols: usize = 0;
        for row in &parsed_rows {
            let sum: usize = row.iter().map(|c| c.col_span as usize).sum();
            if sum > max_cols {
                max_cols = sum;
            }
        }
        max_cols = max_cols.max(1);
        // rowspan 처리를 위한 점유 그리드
        let grid_rows = row_count as usize + 16;
        let grid_cols = max_cols + 16;
        let mut occupied = vec![vec![false; grid_cols]; grid_rows];

        struct CellPos {
            row: u16,
            col: u16,
            col_span: u16,
            row_span: u16,
            parsed_row: usize,
            parsed_col: usize,
        }

        let mut cell_positions: Vec<CellPos> = Vec::new();
        let mut actual_col_count: u16 = 0;

        for (ri, row) in parsed_rows.iter().enumerate() {
            let mut col_cursor: usize = 0;
            for (ci, cell) in row.iter().enumerate() {
                // 이미 점유된 위치 건너뛰기
                while col_cursor < grid_cols && occupied[ri][col_cursor] {
                    col_cursor += 1;
                }
                let col = col_cursor as u16;

                // 점유 표시
                for dr in 0..cell.row_span as usize {
                    for dc in 0..cell.col_span as usize {
                        let r = ri + dr;
                        let c = col_cursor + dc;
                        if r < grid_rows && c < grid_cols {
                            occupied[r][c] = true;
                        }
                    }
                }

                cell_positions.push(CellPos {
                    row: ri as u16,
                    col,
                    col_span: cell.col_span,
                    row_span: cell.row_span,
                    parsed_row: ri,
                    parsed_col: ci,
                });

                let end_col = col + cell.col_span;
                if end_col > actual_col_count {
                    actual_col_count = end_col;
                }
                col_cursor += cell.col_span as usize;
            }
        }

        let col_count = actual_col_count.max(1);

        // --- 3. 셀 크기 계산 ---
        let default_page_width: u32 = 42520; // A4 좌우 여백 제외
        let default_col_width = default_page_width / col_count as u32;
        let default_row_height: u32 = 1000;

        // 열별 폭 (CSS 지정 우선, 없으면 균등 분할)
        let mut col_widths = vec![0u32; col_count as usize];
        for cp in &cell_positions {
            if cp.col_span == 1 {
                let pc = &parsed_rows[cp.parsed_row][cp.parsed_col];
                if pc.width_pt > 0.0 {
                    let w = (pc.width_pt * 100.0).round() as u32;
                    if w > col_widths[cp.col as usize] {
                        col_widths[cp.col as usize] = w;
                    }
                }
            }
        }
        for w in col_widths.iter_mut() {
            if *w == 0 {
                *w = default_col_width;
            }
        }

        // 행별 높이: 명시 height 우선, 없으면 셀 내용의 줄바꿈 수를 추정해 자동 확장한다.
        // (HTML로 만든 표는 저장 높이를 고정값으로 쓰므로 레이아웃이 다시 키우지 않는다.
        //  그래서 여러 줄로 접히는 셀이 1줄 높이에 잘리는 것을 방지하려면 여기서 추정한다.)
        let mut row_heights = vec![0u32; row_count as usize];
        for cp in &cell_positions {
            if cp.row_span != 1 {
                continue;
            }
            let pc = &parsed_rows[cp.parsed_row][cp.parsed_col];
            let h = if pc.height_pt > 0.0 {
                (pc.height_pt * 100.0).round() as u32
            } else {
                // 셀 너비(HWPUNIT) — 병합 고려
                let cell_w: u32 = (cp.col..cp.col + cp.col_span)
                    .map(|c| col_widths.get(c as usize).copied().unwrap_or(default_col_width))
                    .sum();
                estimate_cell_height(&pc.content_html, cell_w, default_row_height)
            };
            if h > row_heights[cp.row as usize] {
                row_heights[cp.row as usize] = h;
            }
        }
        for h in row_heights.iter_mut() {
            if *h == 0 {
                *h = default_row_height;
            }
        }

        // --- 4. BorderFill 생성 및 Cell 구조체 조립 ---
        // 셀 BorderFill을 추가하기 전의 원본 개수를 기록한다. 표 전체 기본 BorderFill은
        // 반드시 이 원본(템플릿) 범위에서 골라야 한다 — 그러지 않으면 배경색을 가진 셀
        // (예: 어두운 머리행)의 BorderFill이 인덱스 3을 차지해 표 전체 배경으로 번진다.
        let base_border_fills_len = self.document.doc_info.border_fills.len();
        let mut cells: Vec<Cell> = Vec::new();
        let mut has_header_row = false;
        // 정렬별 ParaShape id 캐시: css_to_para_shape_id 는 전체 para_shapes 선형 대조를
        // 하므로, 같은 정렬을 쓰는 수백 셀에서 매번 호출하지 않도록 4종을 메모이즈한다.
        let mut align_ps_cache: std::collections::HashMap<String, u16> =
            std::collections::HashMap::new();

        for cp in &cell_positions {
            let pc = &parsed_rows[cp.parsed_row][cp.parsed_col];

            // 셀 폭/높이 (병합 고려)
            let cell_width: u32 = (cp.col..cp.col + cp.col_span)
                .map(|c| {
                    col_widths
                        .get(c as usize)
                        .copied()
                        .unwrap_or(default_col_width)
                })
                .sum();
            let cell_height: u32 = (cp.row..cp.row + cp.row_span)
                .map(|r| {
                    row_heights
                        .get(r as usize)
                        .copied()
                        .unwrap_or(default_row_height)
                })
                .sum();

            // BorderFill 생성/재사용
            let border_fill_id = self.create_border_fill_from_css(
                &pc.border_widths_pt,
                &pc.border_colors,
                &pc.border_styles,
                pc.background_color,
            );

            // 패딩 (pt → HWPUNIT16, CSS 미지정 시 기본 1.4mm ≈ 397 HWPUNIT)
            let default_pad: f64 = 141.0; // ~0.5mm HWPUNIT
            let padding = crate::model::Padding {
                left: if pc.padding_pt[0] > 0.01 {
                    (pc.padding_pt[0] * 100.0).round() as i16
                } else {
                    default_pad as i16
                },
                right: if pc.padding_pt[1] > 0.01 {
                    (pc.padding_pt[1] * 100.0).round() as i16
                } else {
                    default_pad as i16
                },
                top: if pc.padding_pt[2] > 0.01 {
                    (pc.padding_pt[2] * 100.0).round() as i16
                } else {
                    default_pad as i16
                },
                bottom: if pc.padding_pt[3] > 0.01 {
                    (pc.padding_pt[3] * 100.0).round() as i16
                } else {
                    default_pad as i16
                },
            };

            // 셀 내용 파싱
            // &nbsp; 등 HTML 엔티티를 디코딩한 후 공백만 남으면 빈 셀로 처리.
            // 단, 이미지/중첩표만 있는 셀은 plain-text 가 비어도 빈 셀이 아니다
            // (예: 사진·통장/신분증 사본 셀 — <img>만 담김). 이 예외가 없으면 셀 이미지가 유실됐다.
            let cell_has_object = contains_tag_ci(&pc.content_html, &["img", "table"]);
            let cell_paragraphs = if !cell_has_object
                && (pc.content_html.trim().is_empty()
                    || html_to_plain_text(&pc.content_html).is_empty())
            {
                vec![Paragraph::new_empty()]
            } else {
                let parsed = self.parse_html_to_paragraphs(&pc.content_html);
                if parsed.is_empty()
                    || parsed
                        .iter()
                        .all(|p| p.text.trim().is_empty() && p.controls.is_empty())
                {
                    vec![Paragraph::new_empty()]
                } else {
                    parsed
                }
            };

            // 셀 문단의 para_shape_id: text-align 지정이 있으면 해당 정렬의 ParaShape,
            // 없으면 기본 "본문"(id=0). (DIFF-3: 유효한 참조 보장)
            let cell_para_shape_id: u16 = match &pc.text_align {
                Some(v) => {
                    if let Some(&id) = align_ps_cache.get(v) {
                        id
                    } else {
                        let id = self.css_to_para_shape_id(&format!("text-align:{v}"));
                        align_ps_cache.insert(v.clone(), id);
                        id
                    }
                }
                None => 0,
            };

            // 셀 문단 보정: char_count_msb, char_count, para_shape_id, raw_header_extra, line_segs
            let mut cell_paragraphs = cell_paragraphs;
            for cp_para in &mut cell_paragraphs {
                cp_para.char_count_msb = true; // 셀 문단은 항상 MSB 설정
                                               // char_count에 문단끝 마커(+1) 포함
                let text_chars = cp_para.text.chars().count() as u32;
                cp_para.char_count = text_chars + 1;

                // para_shape_id: 기본 "본문" ParaShape 사용 (DIFF-3)
                cp_para.para_shape_id = cell_para_shape_id;

                // DIFF-2: char_shapes가 비어있으면 기본 CharShapeRef 추가
                // 모든 셀 문단은 최소 1개의 명시적 CharShapeRef를 가져야 함
                if cp_para.char_shapes.is_empty() {
                    cp_para
                        .char_shapes
                        .push(crate::model::paragraph::CharShapeRef {
                            start_pos: 0,
                            char_shape_id: 0,
                        });
                }

                // raw_header_extra에 instance_id = 0x80000000 설정
                if cp_para.raw_header_extra.len() >= 10 {
                    cp_para.raw_header_extra[6..10].copy_from_slice(&0x80000000u32.to_le_bytes());
                } else {
                    cp_para.raw_header_extra = inline_object_header_extra(
                        cp_para.char_shapes.len() as u16,
                        cp_para.line_segs.len().max(1) as u16,
                    );
                }

                // line_segs: 폰트 크기 기반 높이 계산
                let font_size = cp_para
                    .char_shapes
                    .first()
                    .and_then(|cs| {
                        self.document
                            .doc_info
                            .char_shapes
                            .get(cs.char_shape_id as usize)
                    })
                    .map(|cs| cs.base_size.max(400))
                    .unwrap_or(1000);
                let line_h = font_size;
                let text_h = font_size;
                let baseline = (font_size as f64 * 0.85) as i32;
                let spacing = (font_size as f64 * 0.6) as i32;
                // seg_width: 셀 폭에서 좌우 패딩을 뺀 텍스트 영역 폭
                let seg_w = (cell_width as i32) - (padding.left as i32) - (padding.right as i32);
                // tag(flags): bit 17(first segment) + bit 18(last segment).
                let line_tag: u32 = crate::model::paragraph::LineSeg::TAG_SINGLE_SEGMENT_LINE;

                if cp_para.line_segs.is_empty() {
                    // line_height=0 으로 두어 레이아웃의 `needs_line_seg_reflow`가 참이 되게 한다.
                    // 그래야 paginate 시 셀 안쪽 폭 기준으로 재줄바꿈(reflow_line_segs)되어
                    // 여러 줄로 접히는 셀 내용이 한 줄로 넘쳐 잘리지 않는다. 실제 line_height는
                    // reflow가 폰트 메트릭으로 재계산한다. (segment_width는 fresh 렌더용으로 유지.)
                    cp_para.line_segs.push(crate::model::paragraph::LineSeg {
                        text_start: 0,
                        line_height: 0,
                        text_height: 0,
                        baseline_distance: 0,
                        line_spacing: 0,
                        segment_width: seg_w,
                        tag: line_tag,
                        ..Default::default()
                    });
                } else {
                    for ls in &mut cp_para.line_segs {
                        if ls.line_height < font_size {
                            ls.line_height = line_h;
                            ls.text_height = text_h;
                            ls.baseline_distance = baseline;
                            ls.line_spacing = spacing;
                        }
                        if ls.segment_width == 0 {
                            ls.segment_width = seg_w;
                        }
                        if ls.tag == 0 {
                            ls.tag = line_tag;
                        }
                    }
                }
            }

            if pc.is_header {
                has_header_row = true;
            }

            // list_header_width_ref: is_header면 bit 2 설정
            let lh_width_ref: u16 = if pc.is_header { 0x04 } else { 0 };

            // vertical_align → VerticalAlign enum
            // CSS에서 지정하지 않으면 기본값 Center (정상 HWP 파일 패턴)
            let v_align = match pc.vertical_align {
                0 => crate::model::table::VerticalAlign::Center, // CSS 미지정 → Center (HWP 기본)
                1 => crate::model::table::VerticalAlign::Center,
                2 => crate::model::table::VerticalAlign::Bottom,
                3 => crate::model::table::VerticalAlign::Top, // 명시적 top
                _ => crate::model::table::VerticalAlign::Center,
            };

            // raw_list_extra: 13바이트 (첫 4바이트 = 셀 폭 u32, 나머지 0)
            // 정상 파일에서 raw_list_extra[0..4] = cell_width (u32)
            let mut raw_list_extra = vec![0u8; 13];
            raw_list_extra[0..4].copy_from_slice(&cell_width.to_le_bytes());

            cells.push(Cell {
                col: cp.col,
                row: cp.row,
                col_span: cp.col_span,
                row_span: cp.row_span,
                width: cell_width,
                height: cell_height,
                padding,
                border_fill_id,
                paragraphs: cell_paragraphs,
                is_header: pc.is_header,
                list_header_width_ref: lh_width_ref,
                vertical_align: v_align,
                raw_list_extra,
                ..Default::default()
            });
        }

        // 행 우선 순서로 정렬
        cells.sort_by(|a, b| a.row.cmp(&b.row).then(a.col.cmp(&b.col)));

        // --- 5. Table 구조체 조립 ---
        let total_width: u32 = col_widths.iter().sum();
        let total_height: u32 = row_heights.iter().sum();

        // table.attr: 0x082A2311 에서 bit0(treat_as_char)만 제거한 0x082A2310.
        // treat_as_char 표는 rhwp 조판기가 "인라인 문자"로 취급해 페이지 분할 없이
        // 통째로 배치한다(typeset_tac_table). 그러면 페이지보다 큰 표가 A4를 넘겨도
        // 나뉘지 않아 내용이 잘려 보인다. bit0을 지우면 블록 표(typeset_block_table)로
        // 조판되어 행 경계에서 여러 쪽으로 분할된다(RowBreak). wrap=TopAndBottom(자리차지)·
        // vert_rel=Para 는 유지 — 본문 흐름 안에서 위아래로 배치되는 일반 표.
        let table_attr: u32 = 0x082A2310;

        // raw_ctrl_data: CommonObjAttr 전체 (attr 포함, parse_common_obj_attr 정합)
        // [0..4] attr, [4..8] vertical_offset, [8..12] horizontal_offset,
        // [12..16] width, [16..20] height, [20..24] z_order,
        // [24..26] margin.left, [26..28] margin.right,
        // [28..30] margin.top, [30..32] margin.bottom,
        // [32..36] instance_id, [36..38] desc_len(=0)
        let outer_margin: i16 = 283; // 바깥 여백 ~1mm
        let mut raw_ctrl_data = vec![0u8; 38]; // 32(base) + 2(desc_len) + 4(extra)
        raw_ctrl_data[common_obj_offsets::FLAGS].copy_from_slice(&table_attr.to_le_bytes());
        raw_ctrl_data[common_obj_offsets::WIDTH].copy_from_slice(&total_width.to_le_bytes());
        raw_ctrl_data[common_obj_offsets::HEIGHT].copy_from_slice(&total_height.to_le_bytes());
        // 바깥 여백 (left, right, top, bottom)
        raw_ctrl_data[common_obj_offsets::MARGIN_LEFT].copy_from_slice(&outer_margin.to_le_bytes());
        raw_ctrl_data[common_obj_offsets::MARGIN_RIGHT]
            .copy_from_slice(&outer_margin.to_le_bytes());
        raw_ctrl_data[common_obj_offsets::MARGIN_TOP].copy_from_slice(&outer_margin.to_le_bytes());
        raw_ctrl_data[common_obj_offsets::MARGIN_BOTTOM]
            .copy_from_slice(&outer_margin.to_le_bytes());
        // [32..36] instance_id (DIFF-7 수정: 해시 기반 유니크 값 생성)
        // 정상 HWP 파일에서는 instance_id가 고유한 비-0 값을 가짐
        let instance_id: u32 = {
            // 행/열 수, 셀 수, 총 폭/높이를 조합한 간단한 해시
            let mut h: u32 = 0x7c150000;
            h = h.wrapping_add(row_count as u32 * 0x1000);
            h = h.wrapping_add(col_count as u32 * 0x100);
            h = h.wrapping_add(total_width);
            h = h.wrapping_add(total_height.wrapping_mul(0x1b));
            h ^= cells.len() as u32 * 0x4b69;
            if h == 0 {
                h = 0x7c154b69;
            } // 절대 0이 되지 않도록
            h
        };
        raw_ctrl_data[common_obj_offsets::INSTANCE_ID].copy_from_slice(&instance_id.to_le_bytes());
        // [36..38] desc_len = 0

        // row_sizes: 각 행의 셀 수
        let row_sizes: Vec<i16> = (0..row_count)
            .map(|r| cells.iter().filter(|c| c.row == r).count() as i16)
            .collect();

        // 표 전체 기본 BorderFill: 정상 파일에서 모든 표가 border_fill_id=3 사용.
        // 단, 셀 BorderFill 추가 이전의 원본 개수(base_border_fills_len)로 판단해야
        // 색 배경 셀이 만든 BorderFill을 표 배경으로 오인하지 않는다.
        // border_fill_id는 1-based (DocInfo.border_fills 인덱스 + 1). 원본에 3개 미만이면
        // 배경 없는 id 1(투명)을 기본으로 써서 표 배경 flood를 방지한다.
        let table_border_fill_id = if base_border_fills_len >= 3 {
            3u16
        } else if base_border_fills_len >= 1 {
            1u16
        } else {
            0u16
        };

        // HTML <table> CSS에서 표 패딩 파싱
        let table_style =
            parse_inline_style(&table_html[..table_html.find('>').unwrap_or(table_html.len()) + 1])
                .to_lowercase();
        let table_padding_pt = parse_css_padding_pt(&table_style);
        // 기본값: L:510 R:510 T:141 B:141 (정상 HWP 파일 패턴)
        let table_padding = crate::model::Padding {
            left: if table_padding_pt[0] > 0.01 {
                (table_padding_pt[0] * 100.0).round() as i16
            } else {
                510
            },
            right: if table_padding_pt[1] > 0.01 {
                (table_padding_pt[1] * 100.0).round() as i16
            } else {
                510
            },
            top: if table_padding_pt[2] > 0.01 {
                (table_padding_pt[2] * 100.0).round() as i16
            } else {
                141
            },
            bottom: if table_padding_pt[3] > 0.01 {
                (table_padding_pt[3] * 100.0).round() as i16
            } else {
                141
            },
        };

        // raw_table_record_attr: 정상 파일 패턴 기반 (DIFF-5 수정)
        // bit 1: 셀 분리 금지 (항상 설정), bit 2: repeat_header
        // bit 26: 추가 레이아웃 속성
        // 정상 HWP 파일에서 모든 표는 bit 1 (셀 분리 금지) 이 항상 설정됨
        let tbl_rec_attr: u32 = 0x04000006; // bit 1(셀분리금지) + bit 2 + bit 26

        let outer_margin: i16 = 283; // 바깥 여백 기본값 ~1mm
        let mut table = Table {
            attr: table_attr,
            row_count,
            col_count,
            cell_spacing: 0,
            padding: table_padding,
            row_sizes,
            border_fill_id: table_border_fill_id,
            zones: Vec::new(),
            cells,
            cell_grid: Vec::new(),
            // 행 경계에서 쪽 나눔 허용(RowBreak). None이면 페이지보다 큰 표가 쪽을
            // 넘겨도 분할되지 않아 내용이 A4 밖으로 넘친다. raw_table_record_attr의
            // bit1(셀 분리 금지)과도 정합 — 셀 내부는 나누지 않고 행 경계에서만 나눔.
            page_break: TablePageBreak::RowBreak,
            repeat_header: has_header_row,
            caption: None,
            common: Default::default(),
            outer_margin_left: outer_margin,
            outer_margin_right: outer_margin,
            outer_margin_top: outer_margin,
            outer_margin_bottom: outer_margin,
            raw_ctrl_data,
            raw_table_record_attr: tbl_rec_attr,
            raw_table_record_extra: vec![0u8; 2], // 표준 추가 2바이트
            dirty: true,
            local_resize_rows: Vec::new(),
            local_resize_cols: Vec::new(),
            local_resize_cell_widths: Vec::new(),
            local_resize_cell_heights: Vec::new(),
        };
        table.rebuild_grid();

        // --- 6. Table Control을 포함하는 Paragraph 생성 ---
        // 제어문자는 text에 포함하지 않음 (serialize_para_text가 controls에서 생성)
        let default_char_shape_id = if !self.document.doc_info.char_shapes.is_empty() {
            0u32
        } else {
            self.document
                .doc_info
                .char_shapes
                .push(crate::model::style::CharShape::default());
            0
        };

        // 표 문단의 para_shape_id: 기존 문서의 표 문단에서 사용하는 값 탐색
        // 정상 파일에서 표 문단은 ps_id=1 사용 (기본 "본문" 스타일)
        let table_para_shape_id = {
            let mut found_ps = 0u16;
            'outer: for section in &self.document.sections {
                for para in &section.paragraphs {
                    for ctrl in &para.controls {
                        if let Control::Table(_) = ctrl {
                            found_ps = para.para_shape_id;
                            break 'outer;
                        }
                    }
                }
            }
            // 표 문단은 들여쓰기(margin_left)가 0이어야 표가 본문 좌측에 정렬된다.
            // 템플릿의 ps_id=1은 좌측 여백(예: 3000 HWPUNIT)이 있어 표가 오른쪽으로
            // 밀리므로, margin 0 인 기본 문단 모양(id 0)을 사용한다.
            if found_ps == 0 {
                0u16
            } else {
                found_ps
            }
        };

        let table_raw_header_extra = inline_object_header_extra(1, 1);

        let table_para = Paragraph {
            text: String::new(),
            char_count: 9, // 확장 제어문자(8 code units) + 문단끝(1 code unit)
            control_mask: 0x00000800, // DrawTableObject (bit 11)
            char_offsets: vec![],
            char_shapes: vec![crate::model::paragraph::CharShapeRef {
                start_pos: 0,
                char_shape_id: default_char_shape_id,
            }],
            line_segs: vec![crate::model::paragraph::LineSeg {
                text_start: 0,
                line_height: total_height.min(i32::MAX as u32) as i32,
                text_height: total_height.min(i32::MAX as u32) as i32,
                baseline_distance: (total_height as f64 * 0.85).min(i32::MAX as f64) as i32,
                line_spacing: 600,
                segment_width: total_width.min(i32::MAX as u32) as i32,
                tag: crate::model::paragraph::LineSeg::TAG_SINGLE_SEGMENT_LINE,
                ..Default::default()
            }],
            para_shape_id: table_para_shape_id,
            style_id: 0,
            controls: vec![Control::Table(Box::new(table))],
            ctrl_data_records: vec![None],
            has_para_text: true,
            raw_header_extra: table_raw_header_extra,
            // 표 문단 자체의 MSB는 false (기존 HWP 문서 패턴)
            // FIX-1은 빈 문서 케이스에만 해당, 내용이 있는 문서에서는 false
            // 셀 내부 문단의 MSB는 true (셀 보정 코드에서 설정)
            char_count_msb: false,
            ..Default::default()
        };

        paragraphs.push(table_para);
    }

    /// CSS 테두리/배경 정보로 BorderFill을 생성하고 DocInfo에 등록한다.
    /// 동일한 BorderFill이 이미 있으면 기존 ID를 반환한다.
    pub(crate) fn create_border_fill_from_css(
        &mut self,
        border_widths_pt: &[f64; 4],
        border_colors: &[u32; 4],
        border_styles: &[u8; 4],
        background_color: Option<u32>,
    ) -> u16 {
        use crate::model::style::{
            BorderFill, BorderLine, BorderLineType, DiagonalLine, Fill, FillType, SolidFill,
        };

        let mut borders = [BorderLine::default(); 4];
        for i in 0..4 {
            if border_widths_pt[i] > 0.01 {
                borders[i] = BorderLine {
                    line_type: match border_styles[i] {
                        0 => BorderLineType::None,
                        1 => BorderLineType::Solid,
                        2 => BorderLineType::Dash,
                        3 => BorderLineType::Dot,
                        4 => BorderLineType::Double,
                        _ => BorderLineType::Solid,
                    },
                    width: css_border_width_to_hwp(border_widths_pt[i]),
                    color: border_colors[i],
                };
            } else {
                borders[i] = BorderLine {
                    line_type: BorderLineType::None,
                    width: 0,
                    color: 0,
                };
            }
        }

        let fill = if let Some(bg) = background_color {
            if bg != 0xFFFFFF {
                Fill {
                    fill_type: FillType::Solid,
                    solid: Some(SolidFill {
                        background_color: bg,
                        pattern_color: 0,
                        pattern_type: -1_i32, // 무늬 없음
                    }),
                    ..Default::default()
                }
            } else {
                Fill::default()
            }
        } else {
            Fill::default()
        };

        let bf = BorderFill {
            raw_data: None,
            attr: 0,
            borders,
            diagonal: DiagonalLine::default(),
            fill,
        };

        // 기존 BorderFill에서 동일한 항목 검색
        for (i, existing) in self.document.doc_info.border_fills.iter().enumerate() {
            if border_fills_equal(existing, &bf) {
                return (i + 1) as u16; // border_fill_id는 1-based
            }
        }

        // 새로 추가
        self.document.doc_info.border_fills.push(bf);
        self.document.doc_info.raw_stream_dirty = true;
        self.styles = resolve_styles(&self.document.doc_info, self.dpi);
        self.document.doc_info.border_fills.len() as u16
    }

    /// JSON에서 border/fill 속성을 파싱하여 BorderFill을 생성/재사용한다.
    /// 프론트엔드 글자 테두리/배경 대화상자에서 호출된다.
    pub(crate) fn create_border_fill_from_json(&mut self, json: &str) -> u16 {
        use crate::model::style::{
            BorderFill, BorderLine, DiagonalLine, Fill, FillType, SolidFill,
        };

        // 4방향 테두리 파싱
        let dir_keys = ["borderLeft", "borderRight", "borderTop", "borderBottom"];
        let mut borders = [BorderLine::default(); 4];
        for (i, key) in dir_keys.iter().enumerate() {
            if let Some(obj_str) = json_object(json, key) {
                let type_val = json_i32(&obj_str, "type").unwrap_or(0);
                borders[i].line_type = u8_to_border_line_type(type_val as u8);
                borders[i].width = json_i32(&obj_str, "width").unwrap_or(0) as u8;
                borders[i].color = json_color(&obj_str, "color").unwrap_or(0);
            }
        }

        // 채우기 파싱
        let fill_type_str = json_str(json, "fillType").unwrap_or_default();
        let fill = if fill_type_str == "solid" {
            let bg = json_color(json, "fillColor").unwrap_or(0xFFFFFF);
            let pat_c = json_color(json, "patternColor").unwrap_or(0);
            let pat_t = json_i32(json, "patternType").unwrap_or(0);
            Fill {
                fill_type: FillType::Solid,
                solid: Some(SolidFill {
                    background_color: bg,
                    pattern_color: pat_c,
                    pattern_type: pat_t,
                }),
                ..Default::default()
            }
        } else {
            Fill::default()
        };

        let bf = BorderFill {
            raw_data: None,
            attr: 0,
            borders,
            diagonal: DiagonalLine::default(),
            fill,
        };

        // 기존 BorderFill에서 동일한 항목 검색
        for (i, existing) in self.document.doc_info.border_fills.iter().enumerate() {
            if border_fills_equal(existing, &bf) {
                return (i + 1) as u16;
            }
        }

        // 새로 추가
        self.document.doc_info.border_fills.push(bf);
        self.document.doc_info.raw_stream_dirty = true;
        self.styles = resolve_styles(&self.document.doc_info, self.dpi);
        self.document.doc_info.border_fills.len() as u16
    }

    /// <img> 태그를 파싱하여 이미지 데이터를 문서에 추가한다.
    /// (base64 data URI만 지원)
    pub(crate) fn parse_img_html(&mut self, paragraphs: &mut Vec<Paragraph>, img_tag: &str) {
        // src="data:image/...;base64,..." 추출
        let src = if let Some(src_start) = img_tag.find("src=\"") {
            let after = &img_tag[src_start + 5..];
            if let Some(end) = after.find('"') {
                &after[..end]
            } else {
                return;
            }
        } else if let Some(src_start) = img_tag.find("src='") {
            let after = &img_tag[src_start + 5..];
            if let Some(end) = after.find('\'') {
                &after[..end]
            } else {
                return;
            }
        } else {
            return;
        };

        if !src.starts_with("data:") {
            // 외부 URL 이미지는 처리하지 않음 — 텍스트로 대체
            let mut para = Paragraph::default();
            para.text = "[이미지]".to_string();
            para.char_count = para.text.encode_utf16().count() as u32;
            para.char_offsets = para
                .text
                .chars()
                .scan(0u32, |acc, c| {
                    let off = *acc;
                    *acc += c.len_utf16() as u32;
                    Some(off)
                })
                .collect();
            paragraphs.push(para);
            return;
        }

        // data:image/png;base64,XXXXX 파싱
        let after_data = &src[5..]; // "image/png;base64,XXXXX"
        let base64_start = if let Some(comma) = after_data.find(',') {
            comma + 1
        } else {
            return;
        };
        let base64_str = &after_data[base64_start..];

        use base64::Engine;
        let decoded = match base64::engine::general_purpose::STANDARD.decode(base64_str) {
            Ok(d) => d,
            Err(_) => return,
        };

        if decoded.is_empty() {
            return;
        }

        // BinData로 등록 (바이트 + 메타데이터 둘 다 필요)
        let new_bin_id = (self.document.bin_data_content.len() + 1) as u16;
        let ext = detect_clipboard_image_mime(&decoded)
            .split('/')
            .nth(1)
            .unwrap_or("png")
            .to_string();
        self.document
            .bin_data_content
            .push(crate::model::bin_data::BinDataContent {
                id: new_bin_id,
                data: decoded.clone(),
                extension: ext.clone(),
            });
        // BinData 메타데이터(bin_data_list) 등록 — 이게 없으면 직렬화기가 이미지 스트림을
        // 쓰지 않아 재로드 시 그림이 빈 placeholder로 렌더된다(스테이트리스 MCP 회귀).
        {
            use crate::model::bin_data::{BinData, BinDataCompression, BinDataStatus, BinDataType};
            // 이미지(PNG/JPG)는 이미 압축된 포맷이므로 **비압축(NoCompress)**으로 저장한다.
            // Default 로 두면 직렬화기가 스트림을 deflate 하는데, 한컴은 BIN####.png 스트림을
            // 곧바로 PNG 로 디코드해서 "파일을 읽거나 저장하는데 오류"가 난다.
            // attr 은 정상 HWP 샘플과 동일: type=1(Embedding) | comp=2(비압축) = 0x0021.
            self.document.doc_info.bin_data_list.push(BinData {
                raw_data: None,
                attr: 0x0021,
                data_type: BinDataType::Embedding,
                compression: BinDataCompression::NoCompress,
                status: BinDataStatus::default(),
                abs_path: None,
                rel_path: None,
                storage_id: new_bin_id,
                extension: Some(ext.clone()),
            });
            // DocInfo 재직렬화 강제 — 코드베이스 관례는 스트림을 지우는 게 아니라 dirty 플래그.
            self.document.doc_info.raw_stream_dirty = true;
        }

        // width/height 추출
        let width = parse_html_attr_f64(img_tag, "width").unwrap_or(200.0);
        let height = parse_html_attr_f64(img_tag, "height").unwrap_or(150.0);

        // px → HWPUNIT
        let w_hu = crate::renderer::px_to_hwpunit(width, self.dpi) as u32;
        let h_hu = crate::renderer::px_to_hwpunit(height, self.dpi) as u32;

        // 원본 이미지 픽셀 크기(PNG/JPEG 헤더). crop 영역 계산용.
        let (nat_w_px, nat_h_px) = crate::renderer::svg::parse_image_dimensions(&decoded)
            .unwrap_or((width as u32, height as u32));

        // 이미지 문단: 표 문단과 동일하게 확장 제어문자(8 code unit) 1개를 담는 인라인
        // 객체 문단으로 만든다. text="[이미지]" 같은 placeholder를 넣으면 레이아웃이
        // 이미지를 인라인으로 인식하지 못해 원본 크기로 (0,0)에 그린다.
        let mut para = Paragraph::default();
        para.text = String::new();
        para.char_count = 9; // 확장 제어문자 8 + 문단끝 1
        para.control_mask = 0x00000800; // 그리기 개체(bit 11)
        para.char_offsets = vec![];
        if self.document.doc_info.char_shapes.is_empty() {
            self.document
                .doc_info
                .char_shapes
                .push(crate::model::style::CharShape::default());
        }
        para.char_shapes = vec![crate::model::paragraph::CharShapeRef {
            start_pos: 0,
            char_shape_id: 0,
        }];
        para.has_para_text = true;
        para.raw_header_extra = inline_object_header_extra(1, 1);

        // Picture 컨트롤 생성
        let mut pic = crate::model::image::Picture::default();
        pic.image_attr.bin_data_id = new_bin_id;
        pic.image_attr.effect = crate::model::image::ImageEffect::RealPic;
        // SHAPE_COMPONENT_PICTURE 의 instance_id(0이면 한컴이 그림을 못 붙임). 비-0 값 지정.
        pic.instance_id = make_instance_id(0x62d8_0000, new_bin_id as u32, nat_w_px, nat_h_px);
        pic.common.width = w_hu;
        pic.common.height = h_hu;
        pic.common.vertical_offset = 0;
        pic.common.horizontal_offset = 0;
        // 인라인 이미지: 글자처럼 취급하여 본문 흐름에 배치
        pic.common.treat_as_char = true;
        // instance_id: 정상 HWP 파일의 개체는 고유한 비-0 값을 가진다(0이면 한컴이 거부 가능).
        pic.common.instance_id = make_instance_id(0x7c16_0000, new_bin_id as u32, w_hu, h_hu);
        pic.common.z_order = 0;
        // 크기 규약(정상 HWP 파일 정합): original = 원본 이미지 자연 크기(픽셀×75 HWPUNIT),
        // current/common = 표시 크기. crop·border_x/y 는 원본(자연) 좌표계를 쓴다.
        // (예전엔 original 을 표시 크기로 두고 crop 을 자연 크기로 둬서 crop>original 이
        //  되었고, 한컴이 "잘림 영역이 이미지 범위를 벗어남"으로 파일을 거부했다.)
        let nat_w_hu = (nat_w_px * 75) as i32;
        let nat_h_hu = (nat_h_px * 75) as i32;
        pic.shape_attr.original_width = nat_w_hu as u32;
        pic.shape_attr.original_height = nat_h_hu as u32;
        pic.shape_attr.current_width = w_hu;
        pic.shape_attr.current_height = h_hu;
        pic.shape_attr.render_sx = 1.0;
        pic.shape_attr.render_sy = 1.0;
        // 정상 한컴 그림의 SHAPE_COMPONENT local_file_version 은 1 (기본 0 이면 실제 파일과 불일치).
        pic.shape_attr.local_file_version = 1;
        // crop: 원본 이미지 전체 영역(자름 없음)
        pic.crop = crate::model::image::CropInfo {
            left: 0,
            top: 0,
            right: nat_w_hu,
            bottom: nat_h_hu,
        };
        // 개체 경계 상자(border_x/y): crop 영역 = 원본 좌표계 사각형(TL,TR,BR,BL)
        pic.border_x = [0, nat_w_hu, nat_w_hu, 0];
        pic.border_y = [0, 0, nat_h_hu, nat_h_hu];
        para.controls.push(Control::Picture(Box::new(pic)));

        paragraphs.push(para);
    }
}

/// 표 셀 내용의 줄바꿈 수를 추정해 필요한 셀 높이(HWPUNIT)를 계산한다.
///
/// HTML로 만든 표는 저장 높이를 고정값으로 쓰므로 레이아웃이 다시 키우지 않는다.
/// 그래서 여러 줄로 접히는 셀이 1줄 높이에 잘리는 것을 막으려면 생성 시점에 근사한다.
/// 정밀 폰트 메트릭 대신 한글/CJK/전각 ≈ 1em, 그 외 ≈ 0.55em 근사를 쓴다.
fn estimate_cell_height(content_html: &str, cell_width_hu: u32, min_height: u32) -> u32 {
    let text = html_to_plain_text(content_html);
    if text.trim().is_empty() {
        return min_height;
    }
    let font_pt = {
        let v = parse_css_dimension_pt(&content_html.to_lowercase(), "font-size");
        if v > 0.01 {
            v
        } else {
            10.0
        }
    };
    // 셀 안쪽 폭(pt): 셀 폭(HWPUNIT/100) - 좌우 여백 근사(약 6pt)
    let inner_w_pt = ((cell_width_hu as f64) / 100.0 - 6.0).max(font_pt);
    // 한글/CJK/전각은 1em, 반각/라틴은 0.55em 근사 — 분류는 레이아웃 reflow 가 쓰는
    // 것과 같은 판정(is_cjk_char)을 재사용해 생성 시 추정과 reflow 결과의 괴리를 줄인다.
    let char_em = |c: char| -> f64 {
        if crate::renderer::layout::is_cjk_char(c) {
            1.0
        } else {
            0.55
        }
    };
    // 논리 줄(명시 개행)별로 wrap 줄 수를 합산
    let mut total_lines = 0u32;
    for logical in text.split('\n') {
        let line_w_pt: f64 = logical.chars().map(|c| char_em(c) * font_pt).sum();
        let lines = (line_w_pt / inner_w_pt).ceil().max(1.0) as u32;
        total_lines += lines;
    }
    let total_lines = total_lines.max(1);
    // 줄 높이(HWPUNIT): reflow가 폰트 크기 약 1.6배로 줄을 쌓으므로 여유 있게 1.6배로 잡아
    // 재줄바꿈된 내용이 행 높이를 넘어 잘리지 않게 한다. + 상하 패딩(약 141*2).
    let line_h = (font_pt * 100.0 * 1.6) as u32;
    (total_lines * line_h + 282).max(min_height)
}

/// 확장 제어문자 호스트 문단(표/그림 등 인라인 개체 문단)의 raw_header_extra 10바이트.
/// 레이아웃: [0..2] n_char_shapes, [2..4] n_range_tags=0, [4..6] n_line_segs,
/// [6..10] instance_id — 정상 파일에서 개체 호스트 문단의 instance_id 는 0x80000000.
fn inline_object_header_extra(n_char_shapes: u16, n_line_segs: u16) -> Vec<u8> {
    let mut rhe = vec![0u8; 10];
    rhe[0..2].copy_from_slice(&n_char_shapes.to_le_bytes());
    rhe[4..6].copy_from_slice(&n_line_segs.to_le_bytes());
    rhe[6..10].copy_from_slice(&0x8000_0000u32.to_le_bytes());
    rhe
}

/// 유일에 가까운 비-0 instance_id 근사 생성. 정상 HWP 파일의 개체 instance_id 는
/// 고유한 비-0 값이다(0이면 한컴이 개체를 못 붙이거나 거부할 수 있음).
/// seed 는 코드 경로 구분용 상위 워드(예: 그림 0x62d8_0000).
fn make_instance_id(seed: u32, bin_id: u32, a: u32, b: u32) -> u32 {
    let h = seed
        .wrapping_add(bin_id.wrapping_mul(0x1_0000))
        .wrapping_add(a.wrapping_mul(0x13))
        .wrapping_add(b.wrapping_mul(0x1b));
    if h == 0 {
        seed | 0x4b69
    } else {
        h
    }
}
