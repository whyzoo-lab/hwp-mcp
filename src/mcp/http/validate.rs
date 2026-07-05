//! HWP/HWPX 출력 바이트의 **구조 검증기**(Axis 3: 표준 완벽 저장).
//!
//! export 로 나가기 전에 파일이 구조적으로 유효한지 — 즉 한컴에서 "열리는 파일"인지 —
//! 기계적으로 확인한다. 렌더/파서가 관대해 통과하던 결함(레코드 경계 어긋남, 스트림 누락,
//! 압축 해제 실패 등 한컴이 "파일을 읽거나 저장하는데 오류"로 거부하는 부류)을 미리 잡는다.
//!
//! 검사 항목(하드=열기 실패 유발 / 소프트=열리지만 이상):
//! - [하드] CFB(OLE) 컨테이너 정상, 필수 스트림(FileHeader/DocInfo/BodyText/Section0) 존재
//! - [하드] FileHeader 시그니처 유효
//! - [하드] DocInfo·각 Section 압축 해제 성공 + 레코드가 스트림 끝과 **정확히** 맞음(경계 무결)
//! - [소프트] 알 수 없는 태그, BinData 레코드 수 ↔ BIN#### 스트림 수 불일치

use std::io::{Cursor, Read};

/// 검증 결과.
pub struct Report {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Report { valid: true, errors: Vec::new(), warnings: Vec::new() }
    }
    fn err(&mut self, m: String) {
        self.valid = false;
        self.errors.push(m);
    }
    fn warn(&mut self, m: String) {
        self.warnings.push(m);
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "valid": self.valid,
            "errors": self.errors,
            "warnings": self.warnings,
        })
    }
}

const HWP_SIGNATURE: &[u8] = b"HWP Document File";
const TAG_MIN: u16 = 0x10;
const TAG_MAX: u16 = 0x5F; // HWPTAG_BEGIN(0x10) + 79
const TAG_BIN_DATA: u16 = 0x12;

/// 포맷 자동 판별 후 검증. hwpx(zip)면 HWPX 검사, 아니면 HWP(CFB) 검사.
pub fn validate(bytes: &[u8]) -> Report {
    if bytes.starts_with(b"PK\x03\x04") {
        validate_hwpx(bytes)
    } else {
        validate_hwp(bytes)
    }
}

/// HWP5(CFB) 바이트 구조 검증.
pub fn validate_hwp(bytes: &[u8]) -> Report {
    let mut r = Report::new();
    let mut comp = match cfb::CompoundFile::open(Cursor::new(bytes)) {
        Ok(c) => c,
        Err(e) => {
            r.err(format!("CFB(OLE) 컨테이너 열기 실패: {e}"));
            return r;
        }
    };

    // 스트림 목록 수집.
    let mut streams: Vec<String> = Vec::new();
    for entry in comp.walk() {
        if entry.is_stream() {
            streams.push(entry.path().to_string_lossy().replace('\\', "/"));
        }
    }
    let has = |name: &str| streams.iter().any(|s| s == name);

    // 필수 스트림.
    for req in ["/FileHeader", "/DocInfo", "/BodyText/Section0"] {
        if !has(req) {
            r.err(format!("필수 스트림 누락: {req}"));
        }
    }

    // FileHeader: 시그니처 + 압축 플래그.
    let mut compressed = false;
    if has("/FileHeader") {
        let mut fh = Vec::new();
        if comp.open_stream("/FileHeader").and_then(|mut s| s.read_to_end(&mut fh)).is_ok() {
            if fh.len() < 256 {
                r.err(format!("FileHeader 길이 부족({}바이트, 최소 256)", fh.len()));
            }
            if fh.len() >= HWP_SIGNATURE.len() && &fh[..HWP_SIGNATURE.len()] != HWP_SIGNATURE {
                r.err("FileHeader 시그니처가 'HWP Document File'이 아님".to_string());
            }
            if fh.len() > 36 {
                compressed = fh[36] & 0x01 != 0;
            }
        } else {
            r.err("FileHeader 스트림 읽기 실패".to_string());
        }
    }

    // DocInfo 레코드 무결.
    let mut bin_data_records = 0usize;
    if has("/DocInfo") {
        if let Some(data) = read_maybe_compressed(&mut comp, "/DocInfo", compressed, &mut r) {
            bin_data_records = walk_records(&data, "DocInfo", &mut r);
        }
    }

    // 모든 BodyText/Section* 레코드 무결.
    let sections: Vec<String> = streams
        .iter()
        .filter(|s| s.starts_with("/BodyText/Section"))
        .cloned()
        .collect();
    for sec in &sections {
        if let Some(data) = read_maybe_compressed(&mut comp, sec, compressed, &mut r) {
            let label = sec.trim_start_matches('/');
            walk_records(&data, label, &mut r);
        }
    }

    // BinData: BIN_DATA 레코드 수 ↔ BinData/BIN#### 스트림 수.
    let bin_streams = streams.iter().filter(|s| s.starts_with("/BinData/")).count();
    if bin_data_records != bin_streams {
        r.warn(format!(
            "BinData 레코드 수({bin_data_records}) ↔ BIN#### 스트림 수({bin_streams}) 불일치 — 그림이 유실/과잉일 수 있음"
        ));
    }

    r
}

/// FileHeader 압축 플래그에 따라 스트림을 (필요시 raw-deflate 해제하여) 반환.
fn read_maybe_compressed(
    comp: &mut cfb::CompoundFile<Cursor<&[u8]>>,
    path: &str,
    compressed: bool,
    r: &mut Report,
) -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    if comp.open_stream(path).and_then(|mut s| s.read_to_end(&mut raw)).is_err() {
        r.err(format!("{path} 스트림 읽기 실패"));
        return None;
    }
    if !compressed {
        return Some(raw);
    }
    let mut out = Vec::new();
    match flate2::read::DeflateDecoder::new(&raw[..]).read_to_end(&mut out) {
        Ok(_) => Some(out),
        Err(e) => {
            r.err(format!("{path} 압축 해제 실패(손상): {e}"));
            None
        }
    }
}

/// 레코드 스트림을 끝까지 걷는다. 헤더 파싱·경계 초과·끝 불일치를 검사하고 BIN_DATA 수를 반환.
fn walk_records(data: &[u8], label: &str, r: &mut Report) -> usize {
    let mut off = 0usize;
    let mut count = 0usize;
    let mut unknown = 0usize;
    let mut bin_data = 0usize;
    while off + 4 <= data.len() {
        let h = u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        let tag = (h & 0x3FF) as u16;
        let mut size = ((h >> 20) & 0xFFF) as usize;
        off += 4;
        if size == 0xFFF {
            if off + 4 > data.len() {
                r.err(format!("{label}: 확장 크기 필드가 스트림을 벗어남(off={off})"));
                return bin_data;
            }
            size = u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
            off += 4;
        }
        if off + size > data.len() {
            r.err(format!(
                "{label}: 레코드(tag=0x{tag:02X}) 크기 {size}가 스트림 경계를 초과(off={off}, len={})",
                data.len()
            ));
            return bin_data;
        }
        if !(TAG_MIN..=TAG_MAX).contains(&tag) {
            unknown += 1;
        }
        if tag == TAG_BIN_DATA {
            bin_data += 1;
        }
        off += size;
        count += 1;
    }
    if off != data.len() {
        r.err(format!(
            "{label}: 레코드 경계가 스트림 끝과 불일치(소비 {off} / 전체 {}) — 레코드 크기 오류",
            data.len()
        ));
    }
    if count == 0 {
        r.warn(format!("{label}: 레코드가 없음"));
    }
    if unknown > 0 {
        r.warn(format!("{label}: 알 수 없는 태그 {unknown}개(스펙 외 레코드)"));
    }
    bin_data
}

/// HWPX(ZIP) 바이트 구조 검증(경량): ZIP 유효 + 필수 엔트리 + section XML well-formed.
pub fn validate_hwpx(bytes: &[u8]) -> Report {
    let mut r = Report::new();
    let mut zip = match zip::ZipArchive::new(Cursor::new(bytes)) {
        Ok(z) => z,
        Err(e) => {
            r.err(format!("HWPX(ZIP) 열기 실패: {e}"));
            return r;
        }
    };
    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();
    // 필수 엔트리(HWPX 패키지 규격).
    for req in ["Contents/content.hpf", "Contents/section0.xml"] {
        if !names.iter().any(|n| n == req) {
            r.err(format!("필수 엔트리 누락: {req}"));
        }
    }
    // mimetype(있으면) 첫 엔트리·무압축 권장(경고 수준).
    if names.iter().any(|n| n == "mimetype") && names.first().map(|s| s.as_str()) != Some("mimetype") {
        r.warn("mimetype 엔트리가 ZIP 첫 항목이 아님(일부 리더가 민감)".to_string());
    }
    // section0.xml well-formed 검사.
    if names.iter().any(|n| n == "Contents/section0.xml") {
        let mut xml = String::new();
        if zip.by_name("Contents/section0.xml").and_then(|mut f| {
            f.read_to_string(&mut xml).map_err(zip::result::ZipError::Io)
        }).is_ok() {
            if let Err(e) = check_xml_well_formed(&xml) {
                r.err(format!("Contents/section0.xml XML 오류: {e}"));
            }
        } else {
            r.err("Contents/section0.xml 읽기 실패".to_string());
        }
    }
    r
}

/// 아주 가벼운 XML well-formed 검사(태그 균형). quick-xml 리더로 끝까지 파싱 성공 여부만 본다.
fn check_xml_well_formed(xml: &str) -> Result<(), String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
        buf.clear();
    }
}
