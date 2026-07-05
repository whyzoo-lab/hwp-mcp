//! MCP 도구 스키마와 핸들러.

use serde_json::{json, Value};

use super::protocol::{INTERNAL_ERROR, INVALID_PARAMS};
use super::session::{detect_format, DocFormat, Session};

/// `tools/list` 결과의 tools 배열.
pub fn tools_list_schema() -> Value {
    json!([
        {
            "name": "open_document",
            "description": "기존 HWP/HWPX 파일을 열어 활성 문서로 만든다. 배포용 문서는 자동으로 편집 가능하게 변환한다.",
            "inputSchema": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "HWP/HWPX 파일 경로"}},
                "required": ["path"]
            }
        },
        {
            "name": "create_document",
            "description": "빈 HWP 문서를 새로 만든다. html을 주면 본문을 그 HTML로 초기화한다(Word→HWP 흐름).",
            "inputSchema": {
                "type": "object",
                "properties": {"html": {"type": "string", "description": "선택: 본문 초기화용 HTML"}}
            }
        },
        {
            "name": "insert_html",
            "description": "현재 문서에 HTML 콘텐츠를 삽입한다. 위치 미지정 시 문서 끝에 추가한다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "html": {"type": "string"},
                    "section": {"type": "integer"},
                    "para": {"type": "integer"},
                    "char_offset": {"type": "integer"}
                },
                "required": ["html"]
            }
        },
        {
            "name": "read_document",
            "description": "현재 문서의 전체 텍스트를 반환한다.",
            "inputSchema": {
                "type": "object",
                "properties": {"format": {"type": "string", "enum": ["markdown", "plain"], "default": "plain"}}
            }
        },
        {
            "name": "search",
            "description": "현재 문서에서 문자열을 검색하여 매치 위치 목록을 반환한다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "case_sensitive": {"type": "boolean", "default": false}
                },
                "required": ["query"]
            }
        },
        {
            "name": "replace_text",
            "description": "검색어를 새 텍스트로 치환한다. all=true면 전체, false면 첫 매치만.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "replacement": {"type": "string"},
                    "all": {"type": "boolean", "default": true},
                    "case_sensitive": {"type": "boolean", "default": false}
                },
                "required": ["query", "replacement"]
            }
        },
        {
            "name": "insert_text",
            "description": "지정한 (section, para, char_offset) 위치에 텍스트를 삽입한다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "section": {"type": "integer"},
                    "para": {"type": "integer"},
                    "char_offset": {"type": "integer"},
                    "text": {"type": "string"}
                },
                "required": ["section", "para", "char_offset", "text"]
            }
        },
        {
            "name": "delete_text",
            "description": "지정한 위치에서 count개의 글자를 삭제한다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "section": {"type": "integer"},
                    "para": {"type": "integer"},
                    "char_offset": {"type": "integer"},
                    "count": {"type": "integer"}
                },
                "required": ["section", "para", "char_offset", "count"]
            }
        },
        {
            "name": "export_document",
            "description": "현재 문서를 지정 경로에 HWP 또는 HWPX로 저장한다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "format": {"type": "string", "enum": ["hwp", "hwpx"], "default": "hwp"}
                },
                "required": ["path"]
            }
        },
        {
            "name": "document_info",
            "description": "현재 활성 문서의 상태(로드 여부, 포맷, 구역/문단 수, 원본 경로)를 반환한다.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "list_fields",
            "description": "현재 문서의 모든 양식 필드(누름틀/필드) 목록과 현재 값을 반환한다.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "get_field_value",
            "description": "이름으로 양식 필드의 현재 값을 조회한다.",
            "inputSchema": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"]
            }
        },
        {
            "name": "set_field_value",
            "description": "이름으로 양식 필드 값을 설정한다(서식 채우기).",
            "inputSchema": {
                "type": "object",
                "properties": {"name": {"type": "string"}, "value": {"type": "string"}},
                "required": ["name", "value"]
            }
        }
    ])
}

/// 구역별 문단 수 목록을 만든다.
fn paragraph_counts(doc: &crate::wasm_api::HwpDocument) -> Vec<u64> {
    let sections = doc.get_section_count() as usize;
    (0..sections)
        .map(|s| doc.get_paragraph_count_native(s).unwrap_or(0) as u64)
        .collect()
}

pub fn open_document(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'path' 문자열이 필요합니다".to_string()))?;

    let data = std::fs::read(path)
        .map_err(|e| (INVALID_PARAMS, format!("파일을 읽을 수 없습니다: {path}: {e}")))?;

    let mut doc = crate::wasm_api::HwpDocument::from_bytes(&data)
        .map_err(|e| (INTERNAL_ERROR, format!("HWP 파싱 실패: {e}")))?;

    let was_distribution = doc.document().header.distribution;
    if was_distribution {
        doc.convert_to_editable_native()
            .map_err(|e| (INTERNAL_ERROR, format!("편집 가능 변환 실패: {e}")))?;
    }

    let fmt = detect_format(path);
    let section_count = doc.get_section_count();

    session.doc = Some(doc);
    session.source_path = Some(path.to_string());
    session.format = Some(fmt);

    Ok(json!({
        "format": fmt.as_str(),
        "section_count": section_count,
        "was_distribution": was_distribution
    }))
}

pub fn document_info(session: &mut Session, _args: &Value) -> Result<Value, (i64, String)> {
    match session.doc.as_ref() {
        None => Ok(json!({"loaded": false})),
        Some(doc) => Ok(json!({
            "loaded": true,
            "format": session.format.map(DocFormat::as_str),
            "source_path": session.source_path,
            "section_count": doc.get_section_count(),
            "paragraph_counts": paragraph_counts(doc)
        })),
    }
}

/// 문서의 끝 위치(마지막 구역, 마지막 문단, 문단 끝 글자오프셋)를 구한다.
pub fn doc_end_position(doc: &crate::wasm_api::HwpDocument) -> (usize, usize, usize) {
    let sections = doc.get_section_count() as usize;
    if sections == 0 {
        return (0, 0, 0);
    }
    let last_sec = sections - 1;
    let para_count = doc.get_paragraph_count_native(last_sec).unwrap_or(1).max(1);
    let last_para = para_count - 1;
    let char_off = doc.get_paragraph_length_native(last_sec, last_para).unwrap_or(0);
    (last_sec, last_para, char_off)
}

/// 빈 HWP 문서를 생성하고 세션에 넣는다(내장 blank2010 템플릿 기반).
fn new_blank_doc() -> Result<crate::wasm_api::HwpDocument, (i64, String)> {
    let mut doc = crate::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native()
        .map_err(|e| (INTERNAL_ERROR, format!("빈 문서 생성 실패: {e}")))?;
    Ok(doc)
}

pub fn create_document(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let mut doc = new_blank_doc()?;

    if let Some(html) = args.get("html").and_then(|v| v.as_str()) {
        if !html.trim().is_empty() {
            doc.paste_html_native(0, 0, 0, html)
                .map_err(|e| (INTERNAL_ERROR, format!("HTML 삽입 실패: {e}")))?;
        }
    }

    let section_count = doc.get_section_count();
    session.doc = Some(doc);
    session.source_path = None;
    session.format = Some(DocFormat::Hwp);

    Ok(json!({"section_count": section_count, "page_count": section_count}))
}

pub fn insert_html(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let html = args.get("html").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'html' 문자열이 필요합니다".to_string()))?;
    if html.trim().is_empty() {
        return Err((INVALID_PARAMS, "빈 HTML 입니다".to_string()));
    }

    let (def_sec, def_para, def_off) = {
        let doc = session.require_doc()?;
        doc_end_position(doc)
    };
    let sec = args.get("section").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(def_sec);
    let para = args.get("para").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(def_para);
    let off = args.get("char_offset").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(def_off);

    let doc = session.require_doc()?;
    let before_paras = doc.get_paragraph_count_native(sec).unwrap_or(0);
    let raw = doc.paste_html_native(sec, para, off, html)
        .map_err(|e| (INTERNAL_ERROR, format!("HTML 삽입 실패: {e}")))?;
    let after_paras = doc.get_paragraph_count_native(sec).unwrap_or(before_paras);

    // paste_html_native는 성공 시 {"ok":true,"paraIdx":N,"charOffset":N},
    // 빈 HTML(파싱 결과 문단 없음) 등 실패 시 {"ok":false,"error":"..."}를 반환한다.
    // 스펙 §4에 맞춰 native 결과의 ok/삽입된 문단 수를 그대로 반영한다(하드코딩된 true 금지).
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| (INTERNAL_ERROR, format!("HTML 삽입 결과 파싱 실패: {e}")))?;
    let ok = parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    // inserted_paragraphs는 "새로 생성된 문단 경계" 개수만 센다. 단일 <p> HTML이
    // 현재 문단에 인라인 병합되는 경우(native가 paraIdx/charOffset만 반환하고 새
    // 문단을 만들지 않는 경우) 이 값은 0일 수 있다 — 내용은 정상 삽입되었으며
    // 그 사실은 ok 필드로 확인한다. .max(1) 같은 클램프로 값을 조작하지 않는다.
    let inserted_paragraphs = if ok { after_paras.saturating_sub(before_paras) } else { 0 };

    Ok(json!({
        "ok": ok,
        "inserted_paragraphs": inserted_paragraphs,
        "section": sec,
        "para": para,
        "char_offset": off
    }))
}

pub fn read_document(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("plain");
    let doc = session.require_doc()?;

    // markdown: 표(| a | b |)·목록·제목까지 포함해 추출(표 셀 내용이 보인다).
    if format == "markdown" {
        let pages = doc.page_count();
        let mut md = String::new();
        for pg in 0..pages {
            if let Ok(page_md) = doc.extract_page_markdown_native(pg) {
                md.push_str(&page_md);
                if !page_md.ends_with('\n') {
                    md.push('\n');
                }
            }
        }
        if !md.trim().is_empty() {
            return Ok(json!({"text": md.trim_end_matches('\n'), "format": "markdown"}));
        }
        // 폴백: 아래 plain.
    }

    let sections = doc.get_section_count() as usize;
    let mut out = String::new();
    for sec in 0..sections {
        let paras = doc.get_paragraph_count_native(sec)
            .map_err(|e| (INTERNAL_ERROR, format!("문단 수 조회 실패: {e}")))?;
        for para in 0..paras {
            let len = doc.get_paragraph_length_native(sec, para)
                .map_err(|e| (INTERNAL_ERROR, format!("문단 길이 조회 실패: {e}")))?;
            let line = if len == 0 {
                String::new()
            } else {
                doc.get_text_range_native(sec, para, 0, len)
                    .map_err(|e| (INTERNAL_ERROR, format!("텍스트 조회 실패: {e}")))?
            };
            out.push_str(&line);
            out.push('\n');
        }
    }

    Ok(json!({"text": out.trim_end_matches('\n'), "format": "plain"}))
}

pub fn search(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let query = args.get("query").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'query' 문자열이 필요합니다".to_string()))?;
    let case_sensitive = args.get("case_sensitive").and_then(|v| v.as_bool()).unwrap_or(false);

    let doc = session.require_doc()?;
    let hits_json = doc.search_all_text_native(query, case_sensitive, true)
        .map_err(|e| (INTERNAL_ERROR, format!("검색 실패: {e}")))?;
    let hits: Value = serde_json::from_str(&hits_json)
        .map_err(|e| (INTERNAL_ERROR, format!("검색 결과 파싱 실패: {e}")))?;

    // search_all_text_native의 실제 필드명은 sec/para/charOffset/length (camelCase).
    // MCP 도구 응답은 section/para/char_offset/length로 정규화한다.
    let normalized: Vec<Value> = hits.as_array().cloned().unwrap_or_default().into_iter().map(|h| {
        json!({
            "section": h.get("sec").or_else(|| h.get("section")).cloned().unwrap_or(json!(0)),
            "para": h.get("para").cloned().unwrap_or(json!(0)),
            "char_offset": h.get("charOffset").or_else(|| h.get("char_offset")).cloned().unwrap_or(json!(0)),
            "length": h.get("length").cloned().unwrap_or(json!(0))
        })
    }).collect();

    Ok(json!({"matches": normalized, "count": normalized.len()}))
}

/// 필수 정수 인자를 usize로 읽는다.
fn u64_arg(args: &Value, key: &str) -> Result<usize, (i64, String)> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or((INVALID_PARAMS, format!("'{key}' 정수 인자가 필요합니다")))
}

pub fn replace_text(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let query = args.get("query").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'query' 문자열이 필요합니다".to_string()))?;
    let replacement = args.get("replacement").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'replacement' 문자열이 필요합니다".to_string()))?;
    let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(true);
    let case_sensitive = args.get("case_sensitive").and_then(|v| v.as_bool()).unwrap_or(false);

    let doc = session.require_doc()?;
    let raw = if all {
        doc.replace_all_native(query, replacement, case_sensitive)
    } else {
        doc.replace_one_native(query, replacement, case_sensitive)
    }
    .map_err(|e| (INTERNAL_ERROR, format!("치환 실패: {e}")))?;

    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| (INTERNAL_ERROR, format!("치환 결과 파싱 실패: {e}")))?;
    // replace_all: {"ok":true,"count":N}; replace_one: {"ok":true|false,...}
    let count = parsed.get("count").and_then(|v| v.as_u64())
        .unwrap_or_else(|| if parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) { 1 } else { 0 });
    Ok(json!({"ok": count > 0, "count": count}))
}

pub fn insert_text(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let char_offset = u64_arg(args, "char_offset")?;
    let text = args.get("text").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'text' 문자열이 필요합니다".to_string()))?;

    let doc = session.require_doc()?;
    // insert_text_native의 현재 실패 경로는 호출자가 준 (section, para, char_offset)이
    // 범위를 벗어난 경우뿐이므로 모든 HwpError를 INVALID_PARAMS로 매핑한다.
    // 이 native 메서드가 이후 입력값과 무관한 실패 모드(예: 내부 상태 손상)를
    // 가지게 되면 이 매핑을 재검토해야 한다.
    doc.insert_text_native(section, para, char_offset, text)
        .map_err(|e| (INVALID_PARAMS, format!("삽입 실패: {e}")))?;
    Ok(json!({"ok": true, "section": section, "para": para, "char_offset": char_offset}))
}

pub fn delete_text(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let char_offset = u64_arg(args, "char_offset")?;
    let count = u64_arg(args, "count")?;

    let doc = session.require_doc()?;
    // delete_text_native도 마찬가지로 좌표(section/para/char_offset/count)가
    // 범위를 벗어난 경우에만 실패하므로 INVALID_PARAMS로 매핑한다.
    // 좌표 외 실패 모드가 추가되면 재검토가 필요하다.
    doc.delete_text_native(section, para, char_offset, count)
        .map_err(|e| (INVALID_PARAMS, format!("삭제 실패: {e}")))?;
    Ok(json!({"ok": true, "section": section, "para": para, "char_offset": char_offset}))
}

pub fn export_document(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let path = args.get("path").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'path' 문자열이 필요합니다".to_string()))?
        .to_string();
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("hwp");

    let doc = session.require_doc()?;
    let bytes = match format {
        "hwpx" => doc.export_hwpx_native()
            .map_err(|e| (INTERNAL_ERROR, format!("HWPX 직렬화 실패: {e}")))?,
        "hwp" => doc.export_hwp_with_adapter()
            .map_err(|e| (INTERNAL_ERROR, format!("HWP 직렬화 실패: {e}")))?,
        other => return Err((INVALID_PARAMS, format!("지원하지 않는 포맷: {other} (hwp|hwpx)"))),
    };

    std::fs::write(&path, &bytes)
        .map_err(|e| (INTERNAL_ERROR, format!("파일 저장 실패: {path}: {e}")))?;

    Ok(json!({"ok": true, "path": path, "format": format, "bytes_len": bytes.len()}))
}

pub fn list_fields(session: &mut Session, _args: &Value) -> Result<Value, (i64, String)> {
    let doc = session.require_doc()?;
    let raw = doc.get_field_list_json();
    let fields: Value = serde_json::from_str(&raw)
        .map_err(|e| (INTERNAL_ERROR, format!("필드 목록 파싱 실패: {e}")))?;
    let count = fields.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({"fields": fields, "count": count}))
}

pub fn get_field_value(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let name = args.get("name").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'name' 문자열이 필요합니다".to_string()))?;
    let doc = session.require_doc()?;
    // get_field_value_by_name의 실패는 호출자가 준 필드 이름이 문서에 없는
    // 경우뿐이므로 INVALID_PARAMS로 매핑한다. 이름 조회 외 실패 모드가
    // 생기면 재검토가 필요하다.
    let raw = doc.get_field_value_by_name(name)
        .map_err(|e| (INVALID_PARAMS, format!("필드 값 조회 실패: {e}")))?;
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| (INTERNAL_ERROR, format!("필드 값 결과 파싱 실패: {e}")))?;
    let value = parsed.get("value").cloned().unwrap_or(Value::Null);
    Ok(json!({"name": name, "value": value}))
}

pub fn set_field_value(session: &mut Session, args: &Value) -> Result<Value, (i64, String)> {
    let name = args.get("name").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'name' 문자열이 필요합니다".to_string()))?;
    let value = args.get("value").and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'value' 문자열이 필요합니다".to_string()))?;
    let doc = session.require_doc()?;
    // set_field_value_by_name 역시 필드 이름을 찾지 못하는 경우에만 실패하므로
    // INVALID_PARAMS로 매핑한다. 이름 조회 외 실패 모드가 생기면 재검토가 필요하다.
    doc.set_field_value_by_name(name, value)
        .map_err(|e| (INVALID_PARAMS, format!("필드 값 설정 실패: {e}")))?;
    Ok(json!({"ok": true, "name": name, "value": value}))
}
