//! Axis 2: 서식 보존 편집 하드닝 회귀.
//!
//! 스테이트리스 MCP 경로(load→edit→save→reload)에서 편집이 **유실 없이 반영**되고,
//! 손대지 않은 내용은 **보존**되며, 출력이 **구조적으로 유효**(Axis 3)한지 검증한다.
//! 핵심 위험은 "raw_stream 캐시 유실" — IR 편집이 섹션 원본 바이트 캐시를 안 비우면
//! 직렬화기가 옛 바이트를 재사용해 편집이 사라진다. 새 편집 명령이 이를 깨면 여기서 잡힌다.
#![cfg(feature = "mcp-http")]
use rhwp::mcp::http::validate;
use rhwp::wasm_api::HwpDocument;

/// html로 만든 문서를 export→reload 하여 "raw_stream 캐시를 가진 임포트 문서"를 만든다.
fn imported_from_html(html: &str) -> HwpDocument {
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(0, 0, 0, html).unwrap();
    let bytes = d.export_hwp_with_adapter().unwrap();
    HwpDocument::from_bytes(&bytes).unwrap() // raw_stream 캐시 보유 상태
}

fn export_reload(d: &mut HwpDocument) -> HwpDocument {
    let bytes = d.export_hwp_with_adapter().unwrap();
    // 편집 결과 파일이 구조적으로 유효해야 한다(Axis 3 게이트).
    let r = validate::validate_hwp(&bytes);
    assert!(r.valid, "편집 후 export가 구조 검증 실패: {:?}", r.errors);
    HwpDocument::from_bytes(&bytes).unwrap()
}

fn body_text(d: &HwpDocument) -> String {
    let mut out = String::new();
    let secs = d.get_section_count() as usize;
    for s in 0..secs {
        let paras = d.get_paragraph_count_native(s).unwrap_or(0);
        for p in 0..paras {
            let len = d.get_paragraph_length_native(s, p).unwrap_or(0);
            if len > 0 {
                if let Ok(t) = d.get_text_range_native(s, p, 0, len) {
                    out.push_str(&t);
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[test]
fn replace_text_persists_after_roundtrip() {
    // 임포트 문서에서 본문 텍스트를 치환 → 저장 → 재로드 시 반영돼야 한다(유실 금지).
    let mut d = imported_from_html(
        "<p><span style=\"font-family:맑은 고딕;font-size:12pt\">원본단어 앞뒤 문장</span></p>\
         <p><span style=\"font-family:맑은 고딕;font-size:11pt\">둘째 문단 유지</span></p>",
    );
    let res = d.replace_all_native("원본단어", "치환됨", true).unwrap();
    assert!(res.contains("\"count\":1"), "치환 1건이어야: {res}");

    let d2 = export_reload(&mut d);
    let text = body_text(&d2);
    assert!(text.contains("치환됨"), "치환 결과가 재로드 후 유실됨: {text:?}");
    assert!(!text.contains("원본단어"), "원본 단어가 남아있음(치환 미반영): {text:?}");
    // 손대지 않은 둘째 문단 보존
    assert!(text.contains("둘째 문단 유지"), "무관 문단이 유실됨: {text:?}");
}

#[test]
fn cell_text_edit_persists_after_roundtrip() {
    // 임포트 문서의 표 셀 텍스트를 바꾸면 저장·재로드 후 반영돼야 한다.
    let mut d = imported_from_html(
        "<p><span style=\"font-family:맑은 고딕;font-size:11pt\">표 앞 문단</span></p>\
         <table><tr>\
         <td style=\"width:100pt;border:0.5pt solid #000000\">셀에이</td>\
         <td style=\"width:100pt;border:0.5pt solid #000000\">셀비</td></tr></table>",
    );
    // 표 위치 찾기: 표를 담은 문단/컨트롤 인덱스.
    let tables_json = d.list_tables_native();
    let v: serde_json::Value = serde_json::from_str(&tables_json).unwrap();
    let t0 = &v[0];
    let (sec, para, ctrl) = (
        t0["section"].as_u64().unwrap() as usize,
        t0["para"].as_u64().unwrap() as usize,
        t0["control"].as_u64().unwrap() as usize,
    );
    d.set_cell_text_native(sec, para, ctrl, 0, 0, "바뀐셀").unwrap();

    let d2 = export_reload(&mut d);
    let md = d2.extract_page_markdown_native(0).unwrap_or_default();
    assert!(md.contains("바뀐셀"), "셀 편집이 재로드 후 유실됨: {md}");
    assert!(!md.contains("셀에이"), "옛 셀 텍스트가 남음: {md}");
    assert!(md.contains("셀비"), "무관 셀이 유실됨: {md}");
    assert!(md.contains("표 앞 문단"), "무관 문단이 유실됨");
}

#[test]
fn real_hwp_edit_preserves_and_persists() {
    // 실제 한컴 파일(진짜 raw_stream 바이트)을 로드→편집→저장→재로드 시:
    // (1) 무편집 재저장은 텍스트/문단수 보존 (raw_stream passthrough)
    // (2) 치환 편집이 반영 + 무관 문단수 보존 + 출력 구조 유효.
    let candidates = ["samples/KTX.hwp", "samples/mel-001.hwp", "samples/field-01.hwp"];
    let path = match candidates.iter().find(|p| std::path::Path::new(p).exists()) {
        Some(p) => *p,
        None => return, // 샘플 없으면 skip
    };
    let orig = std::fs::read(path).unwrap();
    let d = HwpDocument::from_bytes(&orig).unwrap();
    let para0 = d.get_paragraph_count_native(0).unwrap_or(0);
    let full = body_text(&d);
    // 2글자 이상 한글 토큰을 첫 문단에서 찾는다(치환 대상).
    let token: String = full
        .chars()
        .filter(|c| ('\u{AC00}'..='\u{D7A3}').contains(c))
        .take(2)
        .collect();
    if token.chars().count() < 2 {
        return; // 한글 없는 샘플이면 skip
    }

    // (1) 무편집 재저장 보존
    let mut d_noedit = HwpDocument::from_bytes(&orig).unwrap();
    let re = export_reload(&mut d_noedit); // 내부에서 validate 통과 확인
    assert_eq!(
        re.get_paragraph_count_native(0).unwrap_or(0),
        para0,
        "무편집 재저장이 문단 수를 바꿈(raw_stream 보존 실패)"
    );

    // (2) 치환 편집 반영 + 보존
    let mut d_edit = HwpDocument::from_bytes(&orig).unwrap();
    let marker = "★편집표식★";
    d_edit.replace_all_native(&token, marker, true).unwrap();
    let d2 = export_reload(&mut d_edit);
    let t2 = body_text(&d2);
    assert!(t2.contains(marker), "실제 HWP 치환이 재로드 후 유실됨(파일={path})");
    assert_eq!(
        d2.get_paragraph_count_native(0).unwrap_or(0),
        para0,
        "치환이 무관 문단 수를 바꿈(파일={path})"
    );
}

#[test]
fn insert_and_delete_persist_after_roundtrip() {
    let mut d = imported_from_html(
        "<p><span style=\"font-family:맑은 고딕;font-size:11pt\">ABCDE</span></p>",
    );
    // para0 앞에 텍스트 삽입
    d.insert_text_native(0, 0, 0, "머리").unwrap();
    let d2 = export_reload(&mut d);
    assert!(body_text(&d2).contains("머리ABCDE"), "삽입 유실: {:?}", body_text(&d2));

    // 다시 임포트 상태로 만들어 삭제 검증
    let mut d3 = d2;
    d3.delete_text_native(0, 0, 0, 2).unwrap(); // "머리" 삭제
    let d4 = export_reload(&mut d3);
    let t = body_text(&d4);
    assert!(t.contains("ABCDE") && !t.contains("머리"), "삭제 유실: {t:?}");
}
