//! HWP/HWPX 구조 검증기(Axis 3) 테스트: 정상 파일은 valid, 손상 파일은 검출.
#![cfg(feature = "mcp-http")]
use rhwp::mcp::http::validate;
use rhwp::wasm_api::HwpDocument;

fn make_hwp() -> Vec<u8> {
    let mut d = HwpDocument::create_empty();
    d.create_blank_document_native().unwrap();
    d.paste_html_native(
        0,
        0,
        0,
        "<p><span style=\"font-family:맑은 고딕;font-size:11pt\">검증 테스트 문서</span></p>\
         <table><tr><td style=\"width:100pt;border:0.5pt solid #000000\">가</td>\
         <td style=\"width:100pt;border:0.5pt solid #000000\">나</td></tr></table>",
    )
    .unwrap();
    d.export_hwp_with_adapter().unwrap()
}

#[test]
fn valid_hwp_passes() {
    let bytes = make_hwp();
    let r = validate::validate_hwp(&bytes);
    assert!(
        r.valid,
        "정상 생성 HWP는 valid여야 함. errors={:?}",
        r.errors
    );
    assert!(r.errors.is_empty());
}

#[test]
fn non_cfb_bytes_flagged() {
    let r = validate::validate_hwp(b"this is not an OLE compound file at all");
    assert!(!r.valid);
    assert!(r.errors.iter().any(|e| e.contains("CFB") || e.contains("컨테이너")));
}

#[test]
fn corrupted_section_detected() {
    // BodyText/Section0(압축 deflate) 중간 바이트를 훼손 → 압축 해제 실패 또는 레코드 경계 오류 검출.
    use std::io::{Read, Seek, SeekFrom, Write};
    let bytes = make_hwp();
    let mut cfb2 = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    let orig_len;
    {
        let mut s = cfb2.open_stream("/BodyText/Section0").unwrap();
        let mut sec = Vec::new();
        s.read_to_end(&mut sec).unwrap();
        orig_len = sec.len();
        assert!(orig_len > 20, "Section0 이 비어있음");
        // 중간부 16바이트를 0xFF 로 덮어 deflate 스트림/레코드를 깨뜨린다.
        let start = orig_len / 3;
        s.seek(SeekFrom::Start(start as u64)).unwrap();
        s.write_all(&[0xFFu8; 16]).unwrap();
    }
    let broken = cfb2.into_inner().into_inner();

    // 쓰기 반영 확인(회귀 방지): 재오픈 시 Section0 이 훼손돼야 함.
    let mut chk = Vec::new();
    cfb::CompoundFile::open(std::io::Cursor::new(broken.clone()))
        .unwrap()
        .open_stream("/BodyText/Section0")
        .unwrap()
        .read_to_end(&mut chk)
        .unwrap();
    assert_eq!(chk.len(), orig_len, "길이는 동일해야(덮어쓰기)");

    let r = validate::validate_hwp(&broken);
    assert!(
        !r.valid,
        "훼손된 Section0 는 검출돼야 함. errors={:?} warnings={:?}",
        r.errors, r.warnings
    );
    assert!(r.errors.iter().any(|e| e.contains("Section0")));
}
