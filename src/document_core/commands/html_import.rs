//! HTML 붙여넣기 + HTML 파싱 관련 native 메서드

use super::super::helpers::*;
use crate::document_core::DocumentCore;
use crate::error::HwpError;
use crate::model::control::Control;
use crate::model::event::DocumentEvent;
use crate::model::paragraph::Paragraph;
use crate::renderer::style_resolver::resolve_styles;

impl DocumentCore {
    pub fn paste_html_native(
        &mut self,
        section_idx: usize,
        para_idx: usize,
        char_offset: usize,
        html: &str,
    ) -> Result<String, HwpError> {
        if section_idx >= self.document.sections.len() {
            return Err(HwpError::RenderError(format!(
                "구역 {} 범위 초과",
                section_idx
            )));
        }
        if para_idx >= self.document.sections[section_idx].paragraphs.len() {
            return Err(HwpError::RenderError(format!(
                "문단 {} 범위 초과",
                para_idx
            )));
        }

        // HTML 파싱 → 문단 목록 생성
        let parsed_paras = self.parse_html_to_paragraphs(html);
        if parsed_paras.is_empty() {
            return Ok("{\"ok\":false,\"error\":\"empty html\"}".to_string());
        }

        self.document.sections[section_idx].raw_stream = None;

        let clip_count = parsed_paras.len();

        if clip_count == 1 && parsed_paras[0].controls.is_empty() {
            // 단일 문단 텍스트 삽입
            let clip_text = parsed_paras[0].text.clone();
            let clip_char_shapes = parsed_paras[0].char_shapes.clone();
            let clip_char_offsets = parsed_paras[0].char_offsets.clone();
            let new_chars = clip_text.chars().count();

            self.document.sections[section_idx].paragraphs[para_idx]
                .insert_text_at(char_offset, &clip_text);

            self.apply_clipboard_char_shapes(
                section_idx,
                para_idx,
                char_offset,
                &clip_char_shapes,
                &clip_char_offsets,
                new_chars,
            );

            self.reflow_paragraph(section_idx, para_idx);
            self.recompose_paragraph(section_idx, para_idx);
            self.paginate_if_needed();

            let new_offset = char_offset + new_chars;
            self.event_log.push(DocumentEvent::HtmlImported {
                section: section_idx,
                para: para_idx,
            });
            return Ok(format!(
                "{{\"ok\":true,\"paraIdx\":{},\"charOffset\":{}}}",
                para_idx, new_offset
            ));
        }

        // 컨트롤(표/이미지 등)을 포함하는 문단이 있는지 확인
        let has_controls = parsed_paras.iter().any(|p| !p.controls.is_empty());

        if has_controls {
            // 컨트롤 포함 문단은 merge 불가 → 직접 삽입
            let right_half =
                self.document.sections[section_idx].paragraphs[para_idx].split_at(char_offset);

            // 현재 문단 (왼쪽 반)이 비어있으면 첫 번째 파싱 문단으로 대체
            let left_empty = self.document.sections[section_idx].paragraphs[para_idx]
                .text
                .is_empty();

            let mut insert_idx = if left_empty {
                // 빈 왼쪽 문단이 구조 control(SectionDef/ColumnDef)을 보유하면 통째 대체
                // (paragraphs[para_idx] = parsed_paras[0]) 시 그 control이 파괴된다. 그러면
                // 직렬화에서 PAGE_DEF가 유실되어 재로드 문단이 용지 0×0 이 되고, 스테이트리스
                // 렌더 미리보기가 0×0 으로 깨진다. 이 경우엔 merge_from으로 self의 기존
                // control을 유지한 채 첫 파싱 문단의 텍스트/control 을 이어붙인다.
                // 구조 control이 없는 빈 문단(예: 표만 붙이는 경우)은 기존처럼 대체하여
                // 파싱 문단의 원본 헤더 속성(raw_header_extra 등)을 보존한다.
                let orig_has_controls = !self.document.sections[section_idx].paragraphs[para_idx]
                    .controls
                    .is_empty();
                if orig_has_controls {
                    self.document.sections[section_idx].paragraphs[para_idx]
                        .merge_from(&parsed_paras[0]);
                    // merge_from은 para_shape_id를 바꾸지 않으므로 첫 파싱 문단의 문단 모양
                    // (정렬·문단 테두리 등)을 이어받는다. 그러지 않으면 첫 문단이 제목이어도
                    // 템플릿 빈 문단의 문단 모양이 남아 하단 구분선 등이 유실된다.
                    let new_ps = parsed_paras[0].para_shape_id;
                    if new_ps != 0 {
                        self.document.sections[section_idx].paragraphs[para_idx].para_shape_id =
                            new_ps;
                    }
                } else {
                    self.document.sections[section_idx].paragraphs[para_idx] =
                        parsed_paras[0].clone();
                }
                let idx = para_idx + 1;
                for i in 1..clip_count {
                    self.document.sections[section_idx]
                        .paragraphs
                        .insert(idx + i - 1, parsed_paras[i].clone());
                }
                para_idx + clip_count
            } else {
                // 왼쪽 문단에 텍스트 → 파싱 문단들을 그 뒤에 삽입
                let idx = para_idx + 1;
                for i in 0..clip_count {
                    self.document.sections[section_idx]
                        .paragraphs
                        .insert(idx + i, parsed_paras[i].clone());
                }
                para_idx + 1 + clip_count
            };

            // 오른쪽 반이 비어있지 않으면 새 문단으로 추가
            let last_para_idx;
            let merge_point;
            if !right_half.text.is_empty() {
                self.document.sections[section_idx]
                    .paragraphs
                    .insert(insert_idx, right_half);
                last_para_idx = insert_idx;
                merge_point = 0;
            } else {
                last_para_idx = insert_idx - 1;
                // 마지막 문단이 컨트롤 문단이면 그 뒤 위치
                let last = &self.document.sections[section_idx].paragraphs[last_para_idx];
                merge_point = last.text.chars().count();
            }

            for i in para_idx..=last_para_idx {
                self.reflow_paragraph(section_idx, i);
            }

            // 선택적 재구성: 원본 문단 재구성 + 삽입 문단 composed 추가.
            // 정순으로 삽입해야 한다 — insert_composed_paragraph는 Vec::insert라
            // 매 호출 시 인덱스 <= 현재 길이여야 한다. 역순이면 composed가 아직
            // 짧은 상태에서 높은 인덱스에 삽입하려다 패닉한다(텍스트 분기와 동일하게 정순).
            self.recompose_paragraph(section_idx, para_idx);
            for i in para_idx + 1..=last_para_idx {
                self.insert_composed_paragraph(section_idx, i);
            }
            self.paginate_if_needed();

            self.event_log.push(DocumentEvent::HtmlImported {
                section: section_idx,
                para: para_idx,
            });
            return Ok(format!(
                "{{\"ok\":true,\"paraIdx\":{},\"charOffset\":{}}}",
                last_para_idx, merge_point
            ));
        }

        // 다중 문단 삽입 (컨트롤 없는 텍스트만)
        let right_half =
            self.document.sections[section_idx].paragraphs[para_idx].split_at(char_offset);

        self.document.sections[section_idx].paragraphs[para_idx].merge_from(&parsed_paras[0]);

        let mut insert_idx = para_idx + 1;
        for i in 1..clip_count {
            self.document.sections[section_idx]
                .paragraphs
                .insert(insert_idx, parsed_paras[i].clone());
            insert_idx += 1;
        }

        let last_para_idx = insert_idx - 1;
        let merge_point =
            self.document.sections[section_idx].paragraphs[last_para_idx].merge_from(&right_half);

        for i in para_idx..=last_para_idx {
            self.reflow_paragraph(section_idx, i);
        }

        // 선택적 재구성: 원본 문단 재구성 + 삽입 문단 composed 추가
        self.recompose_paragraph(section_idx, para_idx);
        for i in para_idx + 1..=last_para_idx {
            self.insert_composed_paragraph(section_idx, i);
        }
        self.paginate_if_needed();

        self.event_log.push(DocumentEvent::HtmlImported {
            section: section_idx,
            para: para_idx,
        });
        Ok(format!(
            "{{\"ok\":true,\"paraIdx\":{},\"charOffset\":{}}}",
            last_para_idx, merge_point
        ))
    }

    fn normalize_html_paragraphs_for_cell_paste(parsed_paras: Vec<Paragraph>) -> Vec<Paragraph> {
        // 셀 내부에는 Table Control 중첩 불가 → 컨트롤 포함 문단은 텍스트만 추출
        parsed_paras
            .into_iter()
            .map(|mut p| {
                if !p.controls.is_empty() {
                    let text = if p.text.is_empty() || p.text == "\u{0002}" {
                        match p.controls.first() {
                            Some(Control::Table(tbl)) => tbl
                                .cells
                                .iter()
                                .map(|c| {
                                    c.paragraphs
                                        .iter()
                                        .map(|cp| cp.text.clone())
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                })
                                .collect::<Vec<_>>()
                                .join("\t"),
                            _ => String::new(),
                        }
                    } else {
                        p.text.clone()
                    };
                    p.controls.clear();
                    p.text = text;
                    p.char_count = p.text.encode_utf16().count() as u32;
                    p.char_offsets = p
                        .text
                        .chars()
                        .scan(0u32, |acc, c| {
                            let off = *acc;
                            *acc += c.len_utf16() as u32;
                            Some(off)
                        })
                        .collect();
                }
                p
            })
            .collect()
    }

    fn paste_html_paragraphs_into_cell_paragraphs(
        cell_paras: &mut Vec<Paragraph>,
        cell_para_idx: usize,
        char_offset: usize,
        parsed_paras: &[Paragraph],
    ) -> Result<(usize, usize), HwpError> {
        if cell_para_idx >= cell_paras.len() {
            return Err(HwpError::RenderError(format!(
                "셀 문단 {} 범위 초과",
                cell_para_idx
            )));
        }

        let clip_count = parsed_paras.len();
        if clip_count == 1 && parsed_paras[0].controls.is_empty() {
            let clip_text = parsed_paras[0].text.clone();
            let new_chars = clip_text.chars().count();

            cell_paras[cell_para_idx].insert_text_at(char_offset, &clip_text);

            let clip_char_shapes = parsed_paras[0].char_shapes.clone();
            let clip_char_offsets = parsed_paras[0].char_offsets.clone();
            Self::apply_clipboard_char_shapes_to_para(
                &mut cell_paras[cell_para_idx],
                char_offset,
                &clip_char_shapes,
                &clip_char_offsets,
                new_chars,
            );

            return Ok((cell_para_idx, char_offset + new_chars));
        }

        let right_half = cell_paras[cell_para_idx].split_at(char_offset);
        cell_paras[cell_para_idx].merge_from(&parsed_paras[0]);

        let mut insert_idx = cell_para_idx + 1;
        for parsed_para in parsed_paras.iter().skip(1) {
            cell_paras.insert(insert_idx, parsed_para.clone());
            insert_idx += 1;
        }

        let last_para_idx = insert_idx - 1;
        let merge_point = cell_paras[last_para_idx].merge_from(&right_half);
        Ok((last_para_idx, merge_point))
    }

    /// HTML 문자열을 파싱하여 셀 내부 캐럿 위치에 삽입한다.
    pub fn paste_html_in_cell_native(
        &mut self,
        section_idx: usize,
        parent_para_idx: usize,
        control_idx: usize,
        cell_idx: usize,
        cell_para_idx: usize,
        char_offset: usize,
        html: &str,
    ) -> Result<String, HwpError> {
        let parsed_paras = self.parse_html_to_paragraphs(html);
        if parsed_paras.is_empty() {
            return Ok("{\"ok\":false,\"error\":\"empty html\"}".to_string());
        }
        let parsed_paras = Self::normalize_html_paragraphs_for_cell_paste(parsed_paras);

        let (last_para_idx, merge_point) = {
            let section =
                self.document.sections.get_mut(section_idx).ok_or_else(|| {
                    HwpError::RenderError(format!("구역 {} 범위 초과", section_idx))
                })?;
            section.raw_stream = None;
            let para = section.paragraphs.get_mut(parent_para_idx).ok_or_else(|| {
                HwpError::RenderError(format!("문단 {} 범위 초과", parent_para_idx))
            })?;
            let control = para.controls.get_mut(control_idx).ok_or_else(|| {
                HwpError::RenderError(format!("컨트롤 {} 범위 초과", control_idx))
            })?;
            let table = match control {
                Control::Table(t) => t,
                _ => return Err(HwpError::RenderError("표가 아님".to_string())),
            };
            let cell_paras = &mut table
                .cells
                .get_mut(cell_idx)
                .ok_or_else(|| HwpError::RenderError(format!("셀 {} 범위 초과", cell_idx)))?
                .paragraphs;
            Self::paste_html_paragraphs_into_cell_paragraphs(
                cell_paras,
                cell_para_idx,
                char_offset,
                &parsed_paras,
            )?
        };

        for i in cell_para_idx..=last_para_idx {
            self.reflow_cell_paragraph(section_idx, parent_para_idx, control_idx, cell_idx, i);
        }
        if let Some(Control::Table(t)) = self.document.sections[section_idx].paragraphs
            [parent_para_idx]
            .controls
            .get_mut(control_idx)
        {
            t.dirty = true;
        }
        self.mark_section_dirty(section_idx);
        self.paginate_if_needed();

        self.event_log.push(DocumentEvent::HtmlImported {
            section: section_idx,
            para: parent_para_idx,
        });
        Ok(format!(
            "{{\"ok\":true,\"cellParaIdx\":{},\"charOffset\":{}}}",
            last_para_idx, merge_point
        ))
    }

    /// HTML 문자열을 파싱하여 cellPath가 가리키는 중첩 표 셀에 삽입한다.
    pub fn paste_html_in_cell_by_path_native(
        &mut self,
        section_idx: usize,
        parent_para_idx: usize,
        path: &[(usize, usize, usize)],
        char_offset: usize,
        html: &str,
    ) -> Result<String, HwpError> {
        if path.is_empty() {
            return Err(HwpError::RenderError("경로가 비어있습니다".to_string()));
        }

        let parsed_paras = self.parse_html_to_paragraphs(html);
        if parsed_paras.is_empty() {
            return Ok("{\"ok\":false,\"error\":\"empty html\"}".to_string());
        }
        let parsed_paras = Self::normalize_html_paragraphs_for_cell_paste(parsed_paras);

        let cell_para_idx = path[path.len() - 1].2;
        let (last_para_idx, merge_point) = {
            let cell_paras =
                self.get_cell_paragraphs_mut_by_path(section_idx, parent_para_idx, path)?;
            Self::paste_html_paragraphs_into_cell_paragraphs(
                cell_paras,
                cell_para_idx,
                char_offset,
                &parsed_paras,
            )?
        };

        let outer_ctrl = path[0].0;
        self.mark_cell_control_dirty(section_idx, parent_para_idx, outer_ctrl);
        self.document.sections[section_idx].raw_stream = None;
        self.mark_section_dirty(section_idx);
        self.paginate_if_needed();

        self.event_log.push(DocumentEvent::HtmlImported {
            section: section_idx,
            para: parent_para_idx,
        });
        Ok(format!(
            "{{\"ok\":true,\"cellParaIdx\":{},\"charOffset\":{}}}",
            last_para_idx, merge_point
        ))
    }

    // === HTML 파서 ===

    /// HTML 문자열을 파싱하여 Paragraph 목록을 생성한다.
    pub(crate) fn parse_html_to_paragraphs(&mut self, html: &str) -> Vec<Paragraph> {
        let mut paragraphs: Vec<Paragraph> = Vec::new();

        // <!--StartFragment-->...<!--EndFragment--> 영역 추출 (없으면 전체 사용)
        let content = if let Some(start) = html.find("<!--StartFragment-->") {
            let after = &html[start + 20..];
            if let Some(end) = after.find("<!--EndFragment-->") {
                &after[..end]
            } else {
                after
            }
        } else {
            // <body>...</body> 영역 추출 시도
            if let Some(start) = html.find("<body") {
                let after_tag = &html[start..];
                if let Some(gt) = after_tag.find('>') {
                    let inner = &after_tag[gt + 1..];
                    if let Some(end) = inner.find("</body>") {
                        &inner[..end]
                    } else {
                        inner
                    }
                } else {
                    html
                }
            } else {
                html
            }
        };

        // 블록 태그가 전혀 없으면 전체가 인라인 콘텐츠(예: 표 셀 내용
        // `<span style="color:.."><b>텍스트</b></span>`)다. 이 경우 블록 루프의
        // 단순 span 처리는 내부 태그를 그대로 텍스트로 흘려 `<b>`가 리터럴로 남고
        // 색·굵기가 유실된다. parse_inline_content로 직접 파싱해 서식을 보존한다.
        // (contains_tag_ci: 데이터 URI 포함 수 MB 콘텐츠의 to_lowercase 전체 복사 회피)
        let has_any_tag = content.contains('<');
        let has_block_tag = has_any_tag
            && contains_tag_ci(
                content,
                &[
                    "p", "div", "table", "ul", "ol", "li", "br", "h1", "h2", "h3", "h4", "h5",
                    "h6", "img",
                ],
            );
        if has_any_tag && !has_block_tag {
            let mut para = Paragraph::default();
            self.parse_inline_content(&mut para, content);
            if !para.text.trim().is_empty() {
                return vec![para];
            }
        }

        // 최상위 태그 파싱
        let mut pos = 0;
        let chars: Vec<char> = content.chars().collect();
        let len = chars.len();
        let mut pending_text = String::new();

        while pos < len {
            if chars[pos] == '<' {
                // 태그 시작
                let tag_start = pos;
                let tag_end = find_char(&chars, pos, '>');
                if tag_end >= len {
                    break;
                }

                let tag_str: String = chars[tag_start..=tag_end].iter().collect();
                let tag_lower = tag_str.to_lowercase();

                if tag_lower.starts_with("<table") {
                    // 보류 중인 텍스트 처리
                    if !pending_text.trim().is_empty() {
                        self.flush_text_to_paragraphs(&mut paragraphs, &pending_text);
                    }
                    pending_text.clear();

                    // 표 전체 추출
                    let table_end = find_closing_tag_chars(&chars, pos, "table");
                    let table_html: String = chars[tag_start..table_end.min(len)].iter().collect();
                    self.parse_table_html(&mut paragraphs, &table_html);
                    pos = table_end;
                    continue;
                } else if tag_lower.starts_with("<img") {
                    if !pending_text.trim().is_empty() {
                        self.flush_text_to_paragraphs(&mut paragraphs, &pending_text);
                    }
                    pending_text.clear();

                    self.parse_img_html(&mut paragraphs, &tag_str);
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("<p") {
                    // 보류 중인 텍스트 처리
                    if !pending_text.trim().is_empty() {
                        self.flush_text_to_paragraphs(&mut paragraphs, &pending_text);
                    }
                    pending_text.clear();

                    // <p> 블록 추출
                    let p_content_start = tag_end + 1;
                    let p_end = find_closing_tag_chars(&chars, pos, "p");
                    let p_inner: String = chars[p_content_start..p_end.min(len)].iter().collect();
                    // </p> 태그 제거
                    let p_inner = if let Some(idx) = p_inner.rfind("</p>") {
                        &p_inner[..idx]
                    } else {
                        &p_inner
                    };

                    // <p> 내부에 <table> 또는 <img>가 있으면 재귀적으로 처리한다.
                    // parse_inline_content는 표·이미지를 다루지 않으므로, 그대로 두면
                    // <p><img ...></p> 의 그림이 유실된다. 상위 파서가 <img>→Picture,
                    // <table>→Table 컨트롤로 변환한다.
                    if contains_tag_ci(p_inner, &["table", "img"]) {
                        let sub_paras = self.parse_html_to_paragraphs(p_inner);
                        paragraphs.extend(sub_paras);
                        pos = p_end;
                        continue;
                    }

                    let para_style = parse_inline_style(&tag_str);
                    let para_shape_id = self.css_to_para_shape_id(&para_style);

                    let mut para = Paragraph::default();
                    para.para_shape_id = para_shape_id;
                    self.parse_inline_content(&mut para, p_inner);
                    paragraphs.push(para);

                    pos = p_end;
                    continue;
                } else if tag_lower.len() >= 3
                    && tag_lower.starts_with("<h")
                    && tag_lower.as_bytes()[2].is_ascii_digit()
                    && (b'1'..=b'6').contains(&tag_lower.as_bytes()[2])
                {
                    // <h1>~<h6> 제목: 굵게 + 단계별 글자 크기의 별도 문단으로.
                    // (<hr>/<head> 등과 구분하기 위해 세 번째 문자가 1~6 숫자인지 확인)
                    if !pending_text.trim().is_empty() {
                        self.flush_text_to_paragraphs(&mut paragraphs, &pending_text);
                    }
                    pending_text.clear();

                    let level = (tag_lower.as_bytes()[2] - b'0') as usize;
                    let tag_name = format!("h{level}");
                    let h_content_start = tag_end + 1;
                    let h_end = find_closing_tag_chars(&chars, pos, &tag_name);
                    let inner: String = chars[h_content_start..h_end.min(len)].iter().collect();
                    let close = format!("</{tag_name}>");
                    let inner = if let Some(idx) = inner.rfind(&close) {
                        &inner[..idx]
                    } else {
                        &inner
                    };

                    // 제목 글자 크기(pt): h1=18, h2=15, h3=13, h4=12, h5=11, h6=10
                    let size_pt = [18, 15, 13, 12, 11, 10][level - 1];
                    let heading_css = format!("font-size:{size_pt}pt;font-weight:bold");
                    let heading_cs = self.css_to_char_shape_id(&heading_css, true, false, false);

                    let para_style = parse_inline_style(&tag_str);
                    let para_shape_id = self.css_to_para_shape_id(&para_style);

                    let mut para = Paragraph::default();
                    para.para_shape_id = para_shape_id;
                    self.parse_inline_content(&mut para, inner);
                    // 제목 스타일을 기본 런으로 깔고, 내부 인라인 런은 그 뒤에 유지.
                    let inner_runs: Vec<_> = para
                        .char_shapes
                        .drain(..)
                        .filter(|r| r.start_pos > 0)
                        .collect();
                    para.char_shapes.push(crate::model::paragraph::CharShapeRef {
                        start_pos: 0,
                        char_shape_id: heading_cs,
                    });
                    para.char_shapes.extend(inner_runs);
                    paragraphs.push(para);

                    pos = h_end;
                    continue;
                } else if tag_lower.starts_with("<div") {
                    // div 내부의 콘텐츠를 재귀적으로 처리
                    let div_content_start = tag_end + 1;
                    let div_end = find_closing_tag_chars(&chars, pos, "div");
                    let div_inner: String =
                        chars[div_content_start..div_end.min(len)].iter().collect();
                    let div_inner = if let Some(idx) = div_inner.rfind("</div>") {
                        &div_inner[..idx]
                    } else {
                        &div_inner
                    };

                    let sub_paras = self.parse_html_to_paragraphs(div_inner);
                    paragraphs.extend(sub_paras);
                    pos = div_end;
                    continue;
                } else if tag_lower.starts_with("<br") {
                    // <br> → 문단 구분
                    if !pending_text.is_empty() {
                        self.flush_text_to_paragraphs(&mut paragraphs, &pending_text);
                        pending_text.clear();
                    } else {
                        // 빈 문단 추가
                        paragraphs.push(Paragraph::default());
                    }
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("<li") {
                    // 리스트 항목 시작 → 이전 텍스트를 문단으로 확정(항목 간 줄바꿈 보장).
                    // <ul>/<ol> 태그 자체는 아래 '기타 태그 무시'로 건너뛰고,
                    // 각 <li>가 새 문단을 시작하게 한다.
                    if !pending_text.trim().is_empty() {
                        self.flush_text_to_paragraphs(&mut paragraphs, &pending_text);
                    }
                    pending_text.clear();
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("</") {
                    // 닫는 태그 무시
                    pos = tag_end + 1;
                    continue;
                } else {
                    // 기타 태그 무시 (span 등 인라인은 <p> 밖에서 직접 올 수 있음)
                    if tag_lower.starts_with("<span") {
                        // <span>...</span> 인라인 콘텐츠
                        let span_end = find_closing_tag_chars(&chars, pos, "span");
                        let span_full: String =
                            chars[tag_start..span_end.min(len)].iter().collect();
                        let span_full = if let Some(idx) = span_full.rfind("</span>") {
                            &span_full[..idx]
                        } else {
                            &span_full
                        };
                        // span 내부 텍스트 추출. 중첩 인라인 태그(<b>/<i> 등)를 그대로 두면
                        // 리터럴 "<b>텍스트</b>" 로 새므로 태그를 제거한다(이 블록-루프 경로는
                        // 서식 런을 보존하지 못한다 — 굵게/색까지 살리려면 <p> 로 감싸 상위
                        // 문단 파서가 parse_inline_content 경로를 타게 한다).
                        if let Some(gt_pos) = span_full.find('>') {
                            let inner = &span_full[gt_pos + 1..];
                            pending_text.push_str(&decode_html_entities(&html_strip_tags(inner)));
                        }
                        pos = span_end;
                        continue;
                    }
                    pos = tag_end + 1;
                    continue;
                }
            } else {
                // 일반 텍스트
                pending_text.push(chars[pos]);
                pos += 1;
            }
        }

        // 남은 텍스트 처리
        if !pending_text.trim().is_empty() {
            self.flush_text_to_paragraphs(&mut paragraphs, &pending_text);
        }

        // 빈 결과 시 최소 처리
        if paragraphs.is_empty() {
            let plain = html_to_plain_text(html);
            if !plain.is_empty() {
                let mut para = Paragraph::default();
                para.text = plain;
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
            }
        }

        paragraphs
    }

    /// 텍스트를 문단으로 변환하여 추가한다 (줄바꿈 기준 분리).
    pub(crate) fn flush_text_to_paragraphs(&self, paragraphs: &mut Vec<Paragraph>, text: &str) {
        let decoded = decode_html_entities(text);
        for line in decoded.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut para = Paragraph::default();
            para.text = trimmed.to_string();
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
        }
    }

    /// <p> 태그 내부의 인라인 콘텐츠를 파싱하여 Paragraph에 채운다.
    pub(crate) fn parse_inline_content(&mut self, para: &mut Paragraph, html: &str) {
        let mut full_text = String::new();
        // (char_start, char_end, char_shape_id) 형태의 스타일 범위
        let mut style_runs: Vec<(usize, usize, u32)> = Vec::new();

        let chars: Vec<char> = html.chars().collect();
        let len = chars.len();
        let mut pos = 0;

        // 중첩 볼드/이탤릭/밑줄 추적
        let mut inherited_bold = false;
        let mut inherited_italic = false;
        let mut inherited_underline = false;

        while pos < len {
            if chars[pos] == '<' {
                let tag_end = find_char(&chars, pos, '>');
                if tag_end >= len {
                    break;
                }

                let tag_str: String = chars[pos..=tag_end].iter().collect();
                let tag_lower = tag_str.to_lowercase();

                if tag_lower.starts_with("<span") {
                    let span_end_tag = find_closing_tag_chars(&chars, pos, "span");
                    let inner_start = tag_end + 1;
                    let inner_end = {
                        // char 배열에서 "</span>" 검색 (바이트 인덱스 혼동 방지)
                        let close_chars: Vec<char> = "</span>".chars().collect();
                        let mut found = None;
                        for i in inner_start..len.saturating_sub(close_chars.len() - 1) {
                            let slice: String = chars[i..i + close_chars.len().min(len - i)]
                                .iter()
                                .collect();
                            if slice.to_lowercase() == "</span>" {
                                found = Some(i);
                                break;
                            }
                        }
                        found.unwrap_or(span_end_tag)
                    };
                    let inner: String = chars[inner_start..inner_end.min(len)].iter().collect();
                    let inner_text = decode_html_entities(&html_strip_tags(&inner));

                    if !inner_text.is_empty() {
                        // span 내부에 중첩된 <b>/<strong>·<i>/<em>·<u> 서식을 인식해
                        // span의 CSS(색·글꼴 등)와 합쳐 적용한다. 그러지 않으면 tags를
                        // strip하면서 굵게/기울임이 유실된다(예: 표 머리셀 흰 굵은 글자).
                        let inner_lower = inner.to_lowercase();
                        let nested_bold = inner_lower.contains("<b>")
                            || inner_lower.contains("<b ")
                            || inner_lower.contains("<strong");
                        let nested_italic =
                            inner_lower.contains("<i>") || inner_lower.contains("<em");
                        let nested_underline = inner_lower.contains("<u>");
                        let css = parse_inline_style(&tag_str);
                        let char_shape_id = self.css_to_char_shape_id(
                            &css,
                            inherited_bold || nested_bold,
                            inherited_italic || nested_italic,
                            inherited_underline || nested_underline,
                        );
                        let start = full_text.chars().count();
                        full_text.push_str(&inner_text);
                        let end = full_text.chars().count();
                        style_runs.push((start, end, char_shape_id));
                    }

                    pos = span_end_tag;
                    continue;
                } else if tag_lower.starts_with("<b>") || tag_lower.starts_with("<strong") {
                    inherited_bold = true;
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("</b>") || tag_lower.starts_with("</strong") {
                    inherited_bold = false;
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("<i>") || tag_lower.starts_with("<em") {
                    inherited_italic = true;
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("</i>") || tag_lower.starts_with("</em") {
                    inherited_italic = false;
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("<u>") {
                    inherited_underline = true;
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("</u>") {
                    inherited_underline = false;
                    pos = tag_end + 1;
                    continue;
                } else if tag_lower.starts_with("<br") {
                    full_text.push('\n');
                    pos = tag_end + 1;
                    continue;
                } else {
                    // 기타 태그 무시
                    pos = tag_end + 1;
                    continue;
                }
            } else {
                // 태그 밖의 일반 텍스트
                let text_start = pos;
                while pos < len && chars[pos] != '<' {
                    pos += 1;
                }
                let raw: String = chars[text_start..pos].iter().collect();
                let decoded = decode_html_entities(&raw);
                if !decoded.is_empty() {
                    if inherited_bold || inherited_italic || inherited_underline {
                        let css_parts: Vec<String> = [
                            if inherited_bold {
                                Some("font-weight:bold".to_string())
                            } else {
                                None
                            },
                            if inherited_italic {
                                Some("font-style:italic".to_string())
                            } else {
                                None
                            },
                            if inherited_underline {
                                Some("text-decoration:underline".to_string())
                            } else {
                                None
                            },
                        ]
                        .into_iter()
                        .flatten()
                        .collect();
                        let fake_css = css_parts.join(";");
                        let char_shape_id =
                            self.css_to_char_shape_id(&fake_css, false, false, false);
                        let start = full_text.chars().count();
                        full_text.push_str(&decoded);
                        let end = full_text.chars().count();
                        style_runs.push((start, end, char_shape_id));
                    } else {
                        full_text.push_str(&decoded);
                    }
                }
                continue;
            }
        }

        para.text = full_text;
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

        // 스타일 범위를 CharShapeRef로 변환
        for (start, _end, char_shape_id) in &style_runs {
            // char index → UTF-16 위치
            let utf16_pos: u32 = para
                .text
                .chars()
                .take(*start)
                .map(|c| c.len_utf16() as u32)
                .sum();
            para.char_shapes
                .push(crate::model::paragraph::CharShapeRef {
                    start_pos: utf16_pos,
                    char_shape_id: *char_shape_id,
                });
        }
    }

    /// CSS 인라인 스타일 → CharShape ID 변환 (기존에서 검색 또는 신규 생성).
    pub(crate) fn css_to_char_shape_id(
        &mut self,
        css: &str,
        inherited_bold: bool,
        inherited_italic: bool,
        inherited_underline: bool,
    ) -> u32 {
        use crate::model::style::{CharShape, UnderlineType};

        // 기본 CharShape를 기반으로 수정
        let base_id = if !self.document.doc_info.char_shapes.is_empty() {
            0u32
        } else {
            self.document
                .doc_info
                .char_shapes
                .push(CharShape::default());
            0
        };
        let mut cs = self.document.doc_info.char_shapes[base_id as usize].clone();
        // clone은 기본 모양의 raw_data(원본 바이트)까지 복사한다. 직렬화기는 raw_data가
        // 있으면 필드 대신 그 바이트를 그대로 쓰므로, 여기서 비우지 않으면 아래에서
        // 수정한 bold/크기/색이 export 시 전부 유실된다.
        cs.raw_data = None;

        // CSS 속성 파싱 및 적용
        let css_lower = css.to_lowercase();

        // font-family (원본 대소문자 보존을 위해 css 원문에서 값 추출; 없으면 소문자 폴백)
        if let Some(font_name) =
            parse_css_value(css, "font-family").or_else(|| parse_css_value(&css_lower, "font-family"))
        {
            // 여러 폰트를 콤마로 나열한 경우 첫 번째만 사용
            let first = font_name.split(',').next().unwrap_or(&font_name);
            let clean_name = first
                .trim_matches(|c: char| c == '\'' || c == '"')
                .trim()
                .to_string();
            if !clean_name.is_empty() {
                // 문서에 없는 글꼴이면 즉석 등록한다. 그러지 않으면 지정 글꼴이 무시되고
                // 기본 글꼴(함초롬바탕)로 남아, 예컨대 맑은 고딕 지정 문서가 세리프로 렌더된다.
                let font_id = self.find_or_add_font_id(&clean_name);
                cs.font_ids = [font_id; 7];
            }
        }

        // font-size
        if let Some(size_str) = parse_css_value(&css_lower, "font-size") {
            if let Some(pt) = parse_pt_value(&size_str) {
                // pt → HWPUNIT: 1pt = 100 HWPUNIT (base_size 단위)
                cs.base_size = (pt * 100.0) as i32;
            }
        }

        // font-weight
        let is_bold = inherited_bold
            || css_lower.contains("font-weight:bold")
            || css_lower.contains("font-weight: bold")
            || css_lower.contains("font-weight:700")
            || css_lower.contains("font-weight: 700");
        cs.bold = is_bold;

        // font-style
        let is_italic = inherited_italic
            || css_lower.contains("font-style:italic")
            || css_lower.contains("font-style: italic");
        cs.italic = is_italic;

        // color
        if let Some(color_str) = parse_css_value(&css_lower, "color") {
            if let Some(bgr) = css_color_to_hwp_bgr(&color_str) {
                cs.text_color = bgr;
            }
        }

        // text-decoration
        let has_underline = inherited_underline
            || css_lower.contains("text-decoration:underline")
            || css_lower.contains("text-decoration: underline")
            || css_lower.contains("text-decoration-line:underline");
        cs.underline_type = if has_underline {
            UnderlineType::Bottom
        } else {
            UnderlineType::None
        };

        let has_strikethrough = css_lower.contains("text-decoration:line-through")
            || css_lower.contains("text-decoration: line-through")
            || css_lower.contains("line-through");
        cs.strikethrough = has_strikethrough;

        // 동일한 CharShape 검색
        for (i, existing) in self.document.doc_info.char_shapes.iter().enumerate() {
            if *existing == cs {
                return i as u32;
            }
        }

        // 새로 추가
        let new_id = self.document.doc_info.char_shapes.len() as u32;
        self.document.doc_info.char_shapes.push(cs);
        self.document.doc_info.raw_stream_dirty = true;
        // 스타일 세트 갱신
        self.styles = resolve_styles(&self.document.doc_info, self.dpi);
        new_id
    }

    /// CSS 인라인 스타일 → ParaShape ID 변환.
    pub(crate) fn css_to_para_shape_id(&mut self, css: &str) -> u16 {
        use crate::model::style::{Alignment, LineSpacingType};

        if css.is_empty() && !self.document.doc_info.para_shapes.is_empty() {
            return 0;
        }

        let base_id: u16 = 0;
        let mut ps = self
            .document
            .doc_info
            .para_shapes
            .get(base_id as usize)
            .cloned()
            .unwrap_or_default();
        // CharShape와 동일: raw_data가 남아 있으면 직렬화기가 수정 전 바이트를 쓴다.
        ps.raw_data = None;

        let css_lower = css.to_lowercase();

        // text-align
        if let Some(align) = parse_css_value(&css_lower, "text-align") {
            ps.alignment = match align.trim() {
                "left" => Alignment::Left,
                "right" => Alignment::Right,
                "center" => Alignment::Center,
                "justify" => Alignment::Justify,
                _ => ps.alignment,
            };
        }

        // line-height
        if let Some(lh) = parse_css_value(&css_lower, "line-height") {
            let lh = lh.trim();
            if lh.ends_with('%') {
                if let Ok(pct) = lh.trim_end_matches('%').parse::<i32>() {
                    ps.line_spacing = pct;
                    ps.line_spacing_type = LineSpacingType::Percent;
                }
            } else if lh.ends_with("px") {
                if let Ok(px) = lh.trim_end_matches("px").parse::<f64>() {
                    // px → HWPUNIT (1px ≈ 75 HWPUNIT at 96dpi)
                    ps.line_spacing = (px * 7200.0 / 25.4 / (self.dpi / 25.4)).round() as i32;
                    ps.line_spacing_type = LineSpacingType::Fixed;
                }
            }
        }

        // margin-top/bottom → 문단 위/아래 간격(spacing_before/after). 1pt = 100 HWPUNIT.
        if let Some(v) = parse_css_value(&css_lower, "margin-top") {
            if let Some(pt) = parse_pt_value(&v) {
                ps.spacing_before = (pt * 100.0).round() as i32;
            }
        }
        if let Some(v) = parse_css_value(&css_lower, "margin-bottom") {
            if let Some(pt) = parse_pt_value(&v) {
                ps.spacing_after = (pt * 100.0).round() as i32;
            }
        }
        // margin-left/right, text-indent → 문단 여백/들여쓰기 (1pt = 100 HWPUNIT).
        if let Some(v) = parse_css_value(&css_lower, "margin-left") {
            if let Some(pt) = parse_pt_value(&v) {
                ps.margin_left = (pt * 100.0).round() as i32;
            }
        }
        if let Some(v) = parse_css_value(&css_lower, "margin-right") {
            if let Some(pt) = parse_pt_value(&v) {
                ps.margin_right = (pt * 100.0).round() as i32;
            }
        }
        if let Some(v) = parse_css_value(&css_lower, "text-indent") {
            if let Some(pt) = parse_pt_value(&v) {
                ps.indent = (pt * 100.0).round() as i32;
            }
        }

        // 문단 테두리: border / border-top·right·bottom·left (예: 섹션 제목의 하단 구분선).
        // create_border_fill_from_css 인자 순서는 [left, right, top, bottom].
        {
            let mut bw = [0.0f64; 4];
            let mut bc = [0u32; 4];
            let mut bs = [1u8; 4];
            if let Some(v) = parse_css_value(&css_lower, "border") {
                let (w, c, s) = parse_css_border_shorthand(&v);
                for i in 0..4 {
                    bw[i] = w;
                    bc[i] = c;
                    bs[i] = s;
                }
            }
            for (i, side) in ["border-left", "border-right", "border-top", "border-bottom"]
                .iter()
                .enumerate()
            {
                if let Some(v) = parse_css_value(&css_lower, side) {
                    let (w, c, s) = parse_css_border_shorthand(&v);
                    bw[i] = w;
                    bc[i] = c;
                    bs[i] = s;
                }
            }
            if bw.iter().any(|w| *w > 0.01) {
                let bfid = self.create_border_fill_from_css(&bw, &bc, &bs, None);
                ps.border_fill_id = bfid;
                // 텍스트와 테두리 사이 약간의 간격(HWPUNIT). 하단 구분선이 글자에 붙지 않게.
                ps.border_spacing = [0, 0, 60, 60];
            }
        }

        // 동일한 ParaShape 검색
        for (i, existing) in self.document.doc_info.para_shapes.iter().enumerate() {
            if *existing == ps {
                return i as u16;
            }
        }

        let new_id = self.document.doc_info.para_shapes.len() as u16;
        self.document.doc_info.para_shapes.push(ps);
        self.document.doc_info.raw_stream_dirty = true;
        self.styles = resolve_styles(&self.document.doc_info, self.dpi);
        new_id
    }

    /// 폰트 이름으로 font_faces에서 ID를 찾는다.
    pub(crate) fn find_font_id(&self, name: &str) -> Option<u16> {
        let name_lower = name.to_lowercase();
        // 한글 폰트 (인덱스 0)를 먼저, 영어 폰트 (인덱스 1)를 다음으로 검색
        for lang_idx in 0..self.document.doc_info.font_faces.len() {
            for (font_idx, font) in self.document.doc_info.font_faces[lang_idx]
                .iter()
                .enumerate()
            {
                if font.name.to_lowercase() == name_lower {
                    return Some(font_idx as u16);
                }
            }
        }
        None
    }

    /// 폰트 이름으로 ID를 찾고(대소문자 무시), 없으면 등록해 ID를 반환한다.
    ///
    /// 등록은 공용 헬퍼 `find_or_create_font_id_native`에 위임한다 — 7개 언어 배열
    /// 동일 인덱스 등록과 **raw_stream FACE_NAME surgical insert**(스트림 보존 문서에서
    /// 모델-스트림 불일치로 글꼴이 유실되는 것 방지)까지 그쪽이 처리한다.
    pub(crate) fn find_or_add_font_id(&mut self, name: &str) -> u16 {
        if let Some(id) = self.find_font_id(name) {
            return id;
        }
        self.find_or_create_font_id_native(name).max(0) as u16
    }
}
