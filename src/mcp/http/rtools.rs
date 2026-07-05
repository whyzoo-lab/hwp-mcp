//! 원격 도구(doc_id/handle 기반).
use serde_json::{json, Value};
use uuid::Uuid;

use super::doc_ctx;
use super::rpc::{self, tool_text_result, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use super::server::AppState;

/// MCP initialize 응답의 instructions — 어떤 에이전트든 고품질 문서 변환을 재현할 수 있도록
/// 실전 검증된 변환 절차와 HTML 계약, 함정을 안내한다. (한컴2024 실기기 검증 완료 경로)
pub const SERVER_INSTRUCTIONS: &str = r#"이 서버는 HWP(한글) 문서를 생성·편집·변환한다. 다른 형식(docx/PDF/HTML/Markdown 등)의 문서를 HWP로 변환할 때는 아래 절차를 따르면 원본에 가까운 품질이 나온다.

## 변환 절차 (권장 워크플로우)
1. 원본에서 구조·서식 추출: 문단 텍스트, 글꼴/크기(pt)/색/굵기, 정렬, 문단 위아래 여백(pt), 표(열너비 비율·병합·셀배경·테두리), 이미지(원본 바이트), 페이지 여백(mm).
2. 아래 'HTML 계약'에 맞는 인라인 스타일 HTML로 조립.
3. create_document(name, html) 호출.
4. set_page_setup으로 원본 페이지 여백 적용 (아래 '페이지 여백' 참고 — 이 단계를 빠뜨리면 여백이 원본과 크게 달라짐).
5. (필요시) set_header_footer로 머리말/꼬리말, set_page_number로 쪽번호.
6. render_preview(format:"png", page:N)로 페이지를 **이미지로 받아 눈으로 직접 확인**(HWP를 PDF/이미지처럼 정확히 분석·검증) → export_document(format:"hwp")로 다운로드 링크 발급. (기존 HWP를 분석/검증할 때도 format:"png"로 페이지를 보면 정확하다.)
7. 표 내용 검증은 read_document(format:"markdown") 사용(plain은 표 셀이 안 보임).
8. export_document 응답의 `valid`(bool)/`validation`을 확인하라 — 출력 파일의 구조 검증 결과다. `valid:false`(errors 존재)면 한컴에서 안 열릴 수 있으니 원인을 고쳐 재생성하라. 저장 없이 미리 검사하려면 validate_document를 쓴다.

## HTML 계약 (지원 태그·CSS)
- 블록: <p>, <table>/<tr>/<td>, <h1>~<h6>, <li>, <br>. 이미지는 <p><img ...></p> 형태로.
- 인라인 서식: <span style="font-family:글꼴;font-size:NNpt;color:#RRGGBB"> + 내부에 <b>/<i>/<u> 중첩 가능.
- <p> style: text-align(left/center/right/justify), margin-top/bottom/left/right(pt), text-indent(pt), border-top/bottom/left/right(예: "border-bottom:1pt solid #888888" — 제목 밑 구분선).
- <td> style: width(pt, 필수 권장), text-align, background-color:#RRGGBB, border(격자 필요시 셀마다 "border:0.5pt solid #999999"), colspan 속성 지원.
- <img>: src는 data URI만 지원(data:image/png|jpeg;base64,...). 외부 URL 불가. width/height는 px(=pt)로 표시 크기 지정.

## 필수 규칙 (어기면 품질 저하)
- 모든 텍스트는 span으로 감싸 font-family와 font-size를 **항상 명시**하라. 안 하면 기본값(함초롬바탕 10pt)이 적용된다. 문서에 없는 글꼴도 지정하면 즉석 등록된다.
- 셀 안에 여러 줄+서식(굵게 등)이 필요하면 <br> 대신 **줄마다 <p>**를 넣어라. <br> 경로는 서식 태그가 유실될 수 있다.
- 표 열너비(pt) 합계는 본문 폭 이내로: 본문 폭(pt) ≒ (용지폭mm − 좌여백mm − 우여백mm) × 2.835. 기본 여백(좌우 30mm)이면 425pt, 좌우 15mm면 510pt.
- 표에 격자선이 필요하면 각 셀에 border를 명시하라(원본에 테두리가 있는데 생략하면 투명 표가 된다).

## 페이지 여백 (기본값 주의)
- 새 문서 기본: 좌우 30mm, 위 20mm, 아래 15mm, 머리말/꼬리말 각 15mm → 본문 폭 150mm뿐이다. 원본이 다르면 반드시 set_page_setup 호출.
- 본문 상단 시작 = margin_top + margin_header. 원본에 머리말이 없으면 margin_header_mm:0을 주고 margin_top_mm에 원본 위여백을 그대로.
- 원본 여백 추출: docx는 w:pgMar(twip÷56.7=mm), PDF는 텍스트/표 콘텐츠 bbox로 실측.

## 기존 문서 편집
- 기존 HWP 편집: import_document(name)→반환된 upload_url에 파일 PUT→handle로 read/search/replace/format_text/set_paragraph_format/표편집 도구 사용.
- 위치(section/para/char_offset)는 search로 얻는다. 서식 적용은 format_text(굵게/색/크기), 문단 모양은 set_paragraph_format.
- export_document의 다운로드 링크는 24시간 유효(서버 재배포 시 무효 — 재발급하면 됨)."#;

pub fn tools_list_schema() -> Value {
    json!([
        {"name":"create_document","description":"빈 HWP 문서를 생성한다. html을 주면 본문 초기화. 다른 형식(docx/PDF 등)을 HWP로 변환할 때의 HTML 작성 규칙·전체 절차는 서버 instructions 참고(핵심: 모든 텍스트에 font-family/font-size 인라인 명시, 표 셀에 width(pt)·border 명시, 생성 후 set_page_setup으로 원본 여백 적용).",
         "inputSchema":{"type":"object","properties":{"name":{"type":"string"},"html":{"type":"string"}},"required":["name"]}},
        {"name":"list_documents","description":"내 문서 목록(handle/name)을 반환한다.",
         "inputSchema":{"type":"object","properties":{}}},
        {"name":"document_info","description":"문서 상태를 반환한다.",
         "inputSchema":{"type":"object","properties":{"handle":{"type":"string"}}}}
        ,{"name":"read_document","description":"현재/지정 문서 텍스트를 반환한다. format=\"markdown\"이면 표(| a | b |)·목록·제목까지 포함(표 셀 내용 확인 시 사용). plain(기본)은 문단 텍스트만.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"format":{"type":"string","enum":["plain","markdown"]}}}},
        {"name":"search","description":"문서에서 문자열 검색.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"query":{"type":"string"},"case_sensitive":{"type":"boolean"}},"required":["query"]}},
        {"name":"replace_text","description":"검색어를 새 텍스트로 치환.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"query":{"type":"string"},"replacement":{"type":"string"},"all":{"type":"boolean"},"case_sensitive":{"type":"boolean"}},"required":["query","replacement"]}},
        {"name":"insert_text","description":"지정 위치에 텍스트 삽입.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"para":{"type":"integer"},"char_offset":{"type":"integer"},"text":{"type":"string"}},"required":["section","para","char_offset","text"]}},
        {"name":"delete_text","description":"지정 위치에서 count 글자 삭제.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"para":{"type":"integer"},"char_offset":{"type":"integer"},"count":{"type":"integer"}},"required":["section","para","char_offset","count"]}},
        {"name":"insert_html","description":"HTML 콘텐츠를 문서 끝에 삽입. HTML 작성 규칙은 create_document와 동일(서버 instructions 참고).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"html":{"type":"string"}},"required":["html"]}}
        ,{"name":"import_document","description":"업로드용 presigned URL을 발급한다. 클라이언트가 HWP를 올린 뒤 name으로 등록된다.",
          "inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}},
        {"name":"export_document","description":"문서를 지정 포맷으로 저장하고 presigned 다운로드 URL을 반환.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"format":{"type":"string","enum":["hwp","hwpx"]}}}},
        {"name":"list_fields","description":"양식 필드(누름틀) 목록/값.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"}}}},
        {"name":"get_field_value","description":"이름으로 필드 값 조회.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"name":{"type":"string"}},"required":["name"]}},
        {"name":"set_field_value","description":"이름으로 필드 값 설정.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"name":{"type":"string"},"value":{"type":"string"}},"required":["name","value"]}}
        ,{"name":"format_text","description":"지정 범위의 글자 서식을 바꾼다(굵게/기울임/밑줄/취소선/글자크기pt/색 #RRGGBB). 위치는 search 결과의 section/para/char_offset(=start)/length로 얻는다(end=start+length). 하나 이상의 서식 속성 필요.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"para":{"type":"integer"},"start":{"type":"integer"},"end":{"type":"integer"},"bold":{"type":"boolean"},"italic":{"type":"boolean"},"underline":{"type":"boolean"},"strikethrough":{"type":"boolean"},"font_size_pt":{"type":"number"},"color":{"type":"string"}},"required":["section","para","start","end"]}},
        {"name":"set_paragraph_format","description":"문단 서식을 바꾼다. align(left/right/center/justify/distribute), line_spacing(백분율), space_before/space_after/indent/margin_left/margin_right(HWPUNIT). 하나 이상 필요.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"para":{"type":"integer"},"align":{"type":"string","enum":["left","right","center","justify","distribute"]},"line_spacing":{"type":"integer"},"space_before":{"type":"integer"},"space_after":{"type":"integer"},"indent":{"type":"integer"},"margin_left":{"type":"integer"},"margin_right":{"type":"integer"}},"required":["section","para"]}}
        ,{"name":"list_tables","description":"문서의 모든 표를 열거한다. 각 표의 index(표 편집 도구에서 table 인자)·rows·cols 반환.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"}}}},
        {"name":"add_table","description":"문서 끝에 rows×cols 빈 표를 추가한다.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"rows":{"type":"integer"},"cols":{"type":"integer"}},"required":["rows","cols"]}},
        {"name":"set_table_cell_text","description":"표의 (row,col) 셀 텍스트를 통째로 교체한다. table은 list_tables의 index. row/col은 0부터.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"row":{"type":"integer"},"col":{"type":"integer"},"text":{"type":"string"}},"required":["table","row","col","text"]}},
        {"name":"merge_table_cells","description":"표의 (start_row,start_col)~(end_row,end_col) 사각 영역 셀을 병합한다.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"start_row":{"type":"integer"},"start_col":{"type":"integer"},"end_row":{"type":"integer"},"end_col":{"type":"integer"}},"required":["table","start_row","start_col","end_row","end_col"]}},
        {"name":"insert_table_row","description":"표에 행을 삽입한다. row=기준 행, below=true면 아래(기본 true).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"row":{"type":"integer"},"below":{"type":"boolean"}},"required":["table","row"]}},
        {"name":"insert_table_column","description":"표에 열을 삽입한다. col=기준 열, right=true면 오른쪽(기본 true).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"col":{"type":"integer"},"right":{"type":"boolean"}},"required":["table","col"]}},
        {"name":"delete_table_row","description":"표의 row 행을 삭제한다.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"row":{"type":"integer"}},"required":["table","row"]}},
        {"name":"delete_table_column","description":"표의 col 열을 삭제한다.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"col":{"type":"integer"}},"required":["table","col"]}}
        ,{"name":"set_page_setup","description":"페이지 설정(구역 단위, 기본 구역 0). 용지/여백은 mm, 방향은 landscape 불리언. 변경할 값만 지정. 본문 상단 시작 = margin_top + margin_header, 본문 폭 = width - left - right (한컴 기본: 좌우30/위20/아래15/머리말15/꼬리말15).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"landscape":{"type":"boolean"},"width_mm":{"type":"number"},"height_mm":{"type":"number"},"margin_left_mm":{"type":"number"},"margin_right_mm":{"type":"number"},"margin_top_mm":{"type":"number"},"margin_bottom_mm":{"type":"number"},"margin_header_mm":{"type":"number"},"margin_footer_mm":{"type":"number"},"margin_gutter_mm":{"type":"number"}}}},
        {"name":"insert_footnote","description":"본문 (section,para,char_offset) 위치에 각주를 달고 각주 내용(text)을 넣는다.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"para":{"type":"integer"},"char_offset":{"type":"integer"},"text":{"type":"string"}},"required":["section","para","char_offset","text"]}},
        {"name":"add_bookmark","description":"(section,para,char_offset) 위치에 이름표(북마크)를 추가한다.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"para":{"type":"integer"},"char_offset":{"type":"integer"},"name":{"type":"string"}},"required":["section","para","char_offset","name"]}},
        {"name":"list_bookmarks","description":"문서의 북마크 목록(name/sec/para/ctrlIdx/charPos).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"}}}},
        {"name":"delete_bookmark","description":"북마크를 삭제한다. section/para/ctrl_idx는 list_bookmarks의 sec/para/ctrlIdx.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"para":{"type":"integer"},"ctrl_idx":{"type":"integer"}},"required":["section","para","ctrl_idx"]}}
        ,{"name":"set_header_footer","description":"구역(기본 0)의 머리말/꼬리말 텍스트를 설정한다. header_text/footer_text 중 하나 이상. apply_to: 0=양쪽(기본)/1=짝수/2=홀수.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"header_text":{"type":"string"},"footer_text":{"type":"string"},"apply_to":{"type":"integer"}}}},
        {"name":"set_page_number","description":"페이지 번호 필드를 머리말/꼬리말에 넣는다. location: footer(기본)/header.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"section":{"type":"integer"},"location":{"type":"string","enum":["footer","header"]},"apply_to":{"type":"integer"}}}}
        ,{"name":"split_table_cell","description":"표의 (row,col) 셀을 rows×cols로 분할한다. table은 list_tables의 index.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"row":{"type":"integer"},"col":{"type":"integer"},"rows":{"type":"integer"},"cols":{"type":"integer"}},"required":["table","row","col","rows","cols"]}},
        {"name":"add_heading","description":"문서 끝에 제목 문단을 추가한다(굵게+단계별 크기). level 1~6.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"level":{"type":"integer"},"text":{"type":"string"}},"required":["level","text"]}},
        {"name":"get_table_map","description":"특정 표의 셀 격자(row/col/row_span/col_span/text)를 반환한다. table은 list_tables index.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"}},"required":["table"]}},
        {"name":"find_cell_by_label","description":"표에서 label 텍스트를 가진 셀을 찾아 인접 셀(direction: right 기본/down)을 반환한다. set_table_cell_text 채우기에 사용.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"label":{"type":"string"},"direction":{"type":"string","enum":["right","down"]}},"required":["table","label"]}},
        {"name":"list_styles","description":"문단 스타일 목록(index/name/type).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"}}}},
        {"name":"get_document_outline","description":"제목 문단 개요(level/text/section/para).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"}}}},
        {"name":"export_html","description":"문서를 HTML로 반환한다(읽기용).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"}}}}
        ,{"name":"mail_merge","description":"템플릿 문서(치환자 {{키}})와 데이터 행들로 여러 문서를 생성한다. handle=템플릿, rows=[{키:값}, ...]. 생성된 문서 handle 목록 반환(각각 export_document로 다운로드).",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"rows":{"type":"array","items":{"type":"object"}},"name_prefix":{"type":"string"}},"required":["rows"]}},
        {"name":"doc_diff","description":"두 문서를 문단 단위로 비교한다(tag: equal/added/removed + 요약).",
          "inputSchema":{"type":"object","properties":{"handle_a":{"type":"string"},"handle_b":{"type":"string"}},"required":["handle_a","handle_b"]}}
        ,{"name":"table_compute","description":"표의 column 열 숫자 셀을 집계해 (target_row,column) 셀에 넣는다. operation: sum(기본)/average. 숫자는 콤마/단위 제거 후 파싱.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"table":{"type":"integer"},"column":{"type":"integer"},"operation":{"type":"string","enum":["sum","average"]},"target_row":{"type":"integer"}},"required":["table","column","target_row"]}},
        {"name":"create_comparison_table","description":"두 문서(handle_a/b)의 문단 차이로 신구대조표(구분/기존/변경) 문서를 새로 만든다. 생성된 handle 반환.",
          "inputSchema":{"type":"object","properties":{"handle_a":{"type":"string"},"handle_b":{"type":"string"},"name":{"type":"string"}},"required":["handle_a","handle_b"]}}
        ,{"name":"official_lint","description":"공문 작성 규칙을 점검한다. 규칙: 끝 표시('끝.' 종결), 금액 천단위 콤마. 위반 목록·통과여부·점수 반환.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"}}}}
        ,{"name":"build_organization_chart","description":"계층 데이터로 조직도(표) 문서를 만든다. hierarchy: {name, children:[...]} 또는 그 배열. 깊이=열, 노드=행.",
          "inputSchema":{"type":"object","properties":{"hierarchy":{},"name":{"type":"string"}},"required":["hierarchy"]}},
        {"name":"build_image_grid","description":"이미지(base64 data URI)들로 격자 표 문서를 만든다. images:[dataURI 또는 {data,caption}], columns(기본 2).",
          "inputSchema":{"type":"object","properties":{"images":{"type":"array"},"columns":{"type":"integer"},"name":{"type":"string"}},"required":["images"]}},
        {"name":"build_meeting_nameplates","description":"참석자 목록으로 명패(표) 문서를 만든다. participants:[{name,title}], columns(기본 2).",
          "inputSchema":{"type":"object","properties":{"participants":{"type":"array"},"columns":{"type":"integer"},"name":{"type":"string"}},"required":["participants"]}},
        {"name":"repair_hwpx","description":"문서를 파싱→재직렬화하여 정규화된 새 문서를 만든다(구조 문제 복구/정리). 새 handle 반환.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"name":{"type":"string"}}}}
        ,{"name":"validate_document","description":"문서를 지정 포맷(hwp/hwpx)으로 저장했을 때 **구조적으로 유효한지(한컴에서 열리는지)** 검사한다. 레코드 경계 무결·필수 스트림·압축/ZIP 무결·BinData 정합을 확인해 valid(bool)+errors[]+warnings[] 반환. export_document 응답에도 동일 validation이 포함된다.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"format":{"type":"string","enum":["hwp","hwpx"]}}}}
        ,{"name":"render_preview","description":"문서 페이지를 렌더해 미리보기로 반환한다. **format=png: 페이지를 이미지로 반환 → LLM이 눈으로 직접 보고 분석(PDF·이미지 수준의 정확한 시각 분석; HWP 내용/서식/표/색 확인 시 권장)**. format=svg: <text> 기반 SVG 인라인(아티팩트 임베드용). format=pdf: 서버 렌더 PDF 다운로드 URL. page(기본 0)·page_count 포함.",
          "inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"page":{"type":"integer"},"format":{"type":"string","enum":["png","svg","pdf"]}}}}
    ])
}

pub async fn handle_tools_call(
    state: &AppState,
    user: Uuid,
    sid: &str,
    id: Value,
    params: &Value,
) -> Value {
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return rpc::error(id, INVALID_PARAMS, "'name' 누락"),
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result: Result<Value, (i64, String)> = match name {
        "create_document" => create_document(state, user, sid, &args).await,
        "list_documents" => list_documents(state, user, &args).await,
        "document_info" => document_info(state, user, sid, &args).await,
        "read_document" => read_document(state, user, sid, &args).await,
        "search" => search(state, user, sid, &args).await,
        "replace_text" => replace_text(state, user, sid, &args).await,
        "insert_text" => insert_text(state, user, sid, &args).await,
        "delete_text" => delete_text(state, user, sid, &args).await,
        "insert_html" => insert_html(state, user, sid, &args).await,
        "import_document" => import_document(state, user, &args).await,
        "export_document" => export_document(state, user, sid, &args).await,
        "list_fields" => list_fields(state, user, sid, &args).await,
        "get_field_value" => get_field_value(state, user, sid, &args).await,
        "set_field_value" => set_field_value(state, user, sid, &args).await,
        "format_text" => format_text(state, user, sid, &args).await,
        "set_paragraph_format" => set_paragraph_format(state, user, sid, &args).await,
        "list_tables" => list_tables(state, user, sid, &args).await,
        "add_table" => add_table(state, user, sid, &args).await,
        "set_table_cell_text" => set_table_cell_text(state, user, sid, &args).await,
        "merge_table_cells" => merge_table_cells(state, user, sid, &args).await,
        "insert_table_row" => insert_table_row(state, user, sid, &args).await,
        "insert_table_column" => insert_table_column(state, user, sid, &args).await,
        "delete_table_row" => delete_table_row(state, user, sid, &args).await,
        "delete_table_column" => delete_table_column(state, user, sid, &args).await,
        "set_page_setup" => set_page_setup(state, user, sid, &args).await,
        "insert_footnote" => insert_footnote(state, user, sid, &args).await,
        "add_bookmark" => add_bookmark(state, user, sid, &args).await,
        "list_bookmarks" => list_bookmarks(state, user, sid, &args).await,
        "delete_bookmark" => delete_bookmark(state, user, sid, &args).await,
        "set_header_footer" => set_header_footer(state, user, sid, &args).await,
        "set_page_number" => set_page_number(state, user, sid, &args).await,
        "split_table_cell" => split_table_cell(state, user, sid, &args).await,
        "add_heading" => add_heading(state, user, sid, &args).await,
        "get_table_map" => get_table_map(state, user, sid, &args).await,
        "find_cell_by_label" => find_cell_by_label(state, user, sid, &args).await,
        "list_styles" => list_styles(state, user, sid, &args).await,
        "get_document_outline" => get_document_outline(state, user, sid, &args).await,
        "export_html" => export_html(state, user, sid, &args).await,
        "mail_merge" => mail_merge(state, user, sid, &args).await,
        "doc_diff" => doc_diff(state, user, sid, &args).await,
        "table_compute" => table_compute(state, user, sid, &args).await,
        "create_comparison_table" => create_comparison_table(state, user, sid, &args).await,
        "official_lint" => official_lint(state, user, sid, &args).await,
        "build_organization_chart" => build_organization_chart(state, user, sid, &args).await,
        "build_image_grid" => build_image_grid(state, user, sid, &args).await,
        "build_meeting_nameplates" => build_meeting_nameplates(state, user, sid, &args).await,
        "repair_hwpx" => repair_hwpx(state, user, sid, &args).await,
        "render_preview" => render_preview(state, user, sid, &args).await,
        "validate_document" => validate_document(state, user, sid, &args).await,
        _ => Err((METHOD_NOT_FOUND, format!("알 수 없는 도구: {name}"))),
    };
    match result {
        // 도구가 이미 MCP content 배열(예: 이미지)을 반환했으면 그대로 통과, 아니면 텍스트로 감싼다.
        Ok(v) if v.get("content").map(|c| c.is_array()).unwrap_or(false) => rpc::success(id, v),
        Ok(v) => rpc::success(id, tool_text_result(&v)),
        Err((code, msg)) => rpc::error(id, code, &msg),
    }
}

async fn create_document(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'name' 문자열 필요".to_string()))?;
    let mut doc = crate::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native()
        .map_err(|e| (INTERNAL_ERROR, format!("빈 문서 생성 실패: {e}")))?;
    if let Some(html) = args.get("html").and_then(|v| v.as_str()) {
        if !html.trim().is_empty() {
            doc.paste_html_native(0, 0, 0, html)
                .map_err(|e| (INTERNAL_ERROR, format!("HTML 삽입 실패: {e}")))?;
        }
    }
    let bytes = doc
        .export_hwp_with_adapter()
        .map_err(|e| (INTERNAL_ERROR, format!("직렬화 실패: {e}")))?;
    let (doc_id, handle, _etag) =
        doc_ctx::create_stored_doc(state, user, name, &bytes, "hwp").await?;
    state.sessions.set_current_doc(sid, doc_id);
    Ok(json!({"handle": handle, "name": name}))
}

async fn list_documents(
    state: &AppState,
    user: Uuid,
    _args: &Value,
) -> Result<Value, (i64, String)> {
    let rows = state
        .db
        .list_documents(user)
        .await
        .map_err(|e| (INTERNAL_ERROR, e))?;
    let docs: Vec<Value> = rows
        .into_iter()
        .map(|r| json!({"handle": r.handle, "name": r.name, "format": r.format}))
        .collect();
    Ok(json!({"documents": docs, "count": docs.len()}))
}

async fn document_info(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    Ok(json!({"handle": row.handle, "name": row.name, "format": row.format, "etag": row.etag}))
}

fn u64_arg(args: &Value, key: &str) -> Result<usize, (i64, String)> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .ok_or((INVALID_PARAMS, format!("'{key}' 정수 인자 필요")))
}

async fn read_document(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("plain");
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    // markdown: 표를 `| a | b |`로, 목록/제목까지 포함해 추출(표 셀이 보인다).
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
        // markdown 추출이 비면 아래 plain으로 폴백.
    }
    // plain: 문단 텍스트(빠름). 표/개체 내용은 포함하지 않음 — 그건 format="markdown".
    let sections = doc.get_section_count() as usize;
    let mut out = String::new();
    for s in 0..sections {
        let paras = doc
            .get_paragraph_count_native(s)
            .map_err(|e| (INTERNAL_ERROR, format!("문단 수: {e}")))?;
        for p in 0..paras {
            let len = doc
                .get_paragraph_length_native(s, p)
                .map_err(|e| (INTERNAL_ERROR, format!("문단 길이: {e}")))?;
            let line = if len == 0 {
                String::new()
            } else {
                doc.get_text_range_native(s, p, 0, len)
                    .map_err(|e| (INTERNAL_ERROR, format!("텍스트: {e}")))?
            };
            out.push_str(&line);
            out.push('\n');
        }
    }
    Ok(json!({"text": out.trim_end_matches('\n'), "format": "plain"}))
}

async fn search(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'query' 필요".to_string()))?;
    let cs = args
        .get("case_sensitive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let raw = doc
        .search_all_text_native(query, cs, true)
        .map_err(|e| (INTERNAL_ERROR, format!("검색 실패: {e}")))?;
    let hits: Value =
        serde_json::from_str(&raw).map_err(|e| (INTERNAL_ERROR, format!("검색 파싱: {e}")))?;
    let normalized: Vec<Value> = hits
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|h| {
            json!({
                "section": h.get("sec").or_else(|| h.get("section")).cloned().unwrap_or(json!(0)),
                "para": h.get("para").cloned().unwrap_or(json!(0)),
                "char_offset": h.get("charOffset").or_else(|| h.get("char_offset")).cloned().unwrap_or(json!(0)),
                "length": h.get("length").cloned().unwrap_or(json!(0)),
            })
        })
        .collect();
    Ok(json!({"matches": normalized, "count": normalized.len()}))
}

async fn replace_text(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'query' 필요".to_string()))?;
    let repl = args
        .get("replacement")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'replacement' 필요".to_string()))?;
    let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(true);
    let cs = args
        .get("case_sensitive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let raw = if all {
        doc.replace_all_native(query, repl, cs)
    } else {
        doc.replace_one_native(query, repl, cs)
    }
    .map_err(|e| (INTERNAL_ERROR, format!("치환 실패: {e}")))?;
    let parsed: Value =
        serde_json::from_str(&raw).map_err(|e| (INTERNAL_ERROR, format!("치환 파싱: {e}")))?;
    let count = parsed
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| {
            if parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                1
            } else {
                0
            }
        });
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": count > 0, "count": count}))
}

async fn insert_text(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let off = u64_arg(args, "char_offset")?;
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'text' 필요".to_string()))?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    doc.insert_text_native(section, para, off, text)
        .map_err(|e| (INVALID_PARAMS, format!("삽입 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn delete_text(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let off = u64_arg(args, "char_offset")?;
    let count = u64_arg(args, "count")?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    doc.delete_text_native(section, para, off, count)
        .map_err(|e| (INVALID_PARAMS, format!("삭제 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn insert_html(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let html = args
        .get("html")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'html' 필요".to_string()))?;
    if html.trim().is_empty() {
        return Err((INVALID_PARAMS, "빈 HTML".to_string()));
    }
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    // 문서 끝 위치 계산
    let sections = doc.get_section_count() as usize;
    let (sec, para, off) = if sections == 0 {
        (0, 0, 0)
    } else {
        let ls = sections - 1;
        let pc = doc.get_paragraph_count_native(ls).unwrap_or(1).max(1);
        let lp = pc - 1;
        let co = doc.get_paragraph_length_native(ls, lp).unwrap_or(0);
        (ls, lp, co)
    };
    let raw = doc
        .paste_html_native(sec, para, off, html)
        .map_err(|e| (INTERNAL_ERROR, format!("HTML 삽입 실패: {e}")))?;
    let parsed: Value = serde_json::from_str(&raw).unwrap_or(json!({"ok":true}));
    let ok = parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": ok}))
}

async fn format_text(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let start = u64_arg(args, "start")?;
    let end = u64_arg(args, "end")?;
    // MCP 인자를 코어 apply_char_format_native의 props JSON으로 변환.
    let mut props = serde_json::Map::new();
    for key in ["bold", "italic", "underline", "strikethrough"] {
        if let Some(b) = args.get(key).and_then(|v| v.as_bool()) {
            props.insert(key.to_string(), json!(b));
        }
    }
    if let Some(pt) = args.get("font_size_pt").and_then(|v| v.as_f64()) {
        // 코어 fontSize = base_size = pt*100 (예: 15pt → 1500)
        props.insert("fontSize".to_string(), json!((pt * 100.0).round() as i64));
    }
    if let Some(c) = args.get("color").and_then(|v| v.as_str()) {
        props.insert("textColor".to_string(), json!(c));
    }
    if props.is_empty() {
        return Err((
            INVALID_PARAMS,
            "서식 속성(bold/italic/underline/strikethrough/font_size_pt/color) 하나 이상 필요".to_string(),
        ));
    }
    let props_json = Value::Object(props).to_string();
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    doc.apply_char_format_native(section, para, start, end, &props_json)
        .map_err(|e| (INVALID_PARAMS, format!("서식 적용 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn set_paragraph_format(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let mut props = serde_json::Map::new();
    if let Some(a) = args.get("align").and_then(|v| v.as_str()) {
        props.insert("alignment".to_string(), json!(a));
    }
    if let Some(ls) = args.get("line_spacing").and_then(|v| v.as_i64()) {
        // 백분율 줄간격(기존 문단이 대개 Percent이므로 값만 바꿔도 %로 적용)
        props.insert("lineSpacing".to_string(), json!(ls));
    }
    for (arg, key) in [
        ("space_before", "spacingBefore"),
        ("space_after", "spacingAfter"),
        ("indent", "indent"),
        ("margin_left", "marginLeft"),
        ("margin_right", "marginRight"),
    ] {
        if let Some(v) = args.get(arg).and_then(|v| v.as_i64()) {
            props.insert(key.to_string(), json!(v));
        }
    }
    if props.is_empty() {
        return Err((
            INVALID_PARAMS,
            "문단 서식(align/line_spacing/space_before/space_after/indent/margin_left/margin_right) 하나 이상 필요".to_string(),
        ));
    }
    let props_json = Value::Object(props).to_string();
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    doc.apply_para_format_native(section, para, &props_json)
        .map_err(|e| (INVALID_PARAMS, format!("문단 서식 적용 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

/// 표 index(list_tables의 index) → (section, para, control_idx) 리졸버.
fn resolve_table(
    doc: &crate::wasm_api::HwpDocument,
    index: usize,
) -> Result<(usize, usize, usize), (i64, String)> {
    let json = doc.list_tables_native();
    let tables: Value =
        serde_json::from_str(&json).map_err(|e| (INTERNAL_ERROR, format!("표 목록 파싱: {e}")))?;
    let arr = tables.as_array().cloned().unwrap_or_default();
    let t = arr.get(index).ok_or_else(|| {
        (
            INVALID_PARAMS,
            format!("표 인덱스 {index} 없음(문서에 표 {}개)", arr.len()),
        )
    })?;
    Ok((
        t.get("section").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        t.get("para").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        t.get("control").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
    ))
}

async fn list_tables(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let tables: Value = serde_json::from_str(&doc.list_tables_native())
        .map_err(|e| (INTERNAL_ERROR, format!("표 목록 파싱: {e}")))?;
    let count = tables.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({"tables": tables, "count": count}))
}

async fn add_table(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let rows = u64_arg(args, "rows")? as u16;
    let cols = u64_arg(args, "cols")? as u16;
    if rows == 0 || cols == 0 {
        return Err((INVALID_PARAMS, "rows/cols는 1 이상이어야 함".to_string()));
    }
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    // 문서 끝 위치 계산(insert_html과 동일)
    let sections = doc.get_section_count() as usize;
    let (sec, para, off) = if sections == 0 {
        (0, 0, 0)
    } else {
        let ls = sections - 1;
        let pc = doc.get_paragraph_count_native(ls).unwrap_or(1).max(1);
        let lp = pc - 1;
        let co = doc.get_paragraph_length_native(ls, lp).unwrap_or(0);
        (ls, lp, co)
    };
    doc.create_table_native(sec, para, off, rows, cols)
        .map_err(|e| (INTERNAL_ERROR, format!("표 생성 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true, "rows": rows, "cols": cols}))
}

async fn set_table_cell_text(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let cell_row = u64_arg(args, "row")? as u16;
    let cell_col = u64_arg(args, "col")? as u16;
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'text' 필요".to_string()))?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let (sec, para, ctrl) = resolve_table(&doc, table)?;
    doc.set_cell_text_native(sec, para, ctrl, cell_row, cell_col, text)
        .map_err(|e| (INVALID_PARAMS, format!("셀 텍스트 설정 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn merge_table_cells(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let sr = u64_arg(args, "start_row")? as u16;
    let sc = u64_arg(args, "start_col")? as u16;
    let er = u64_arg(args, "end_row")? as u16;
    let ec = u64_arg(args, "end_col")? as u16;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let (sec, para, ctrl) = resolve_table(&doc, table)?;
    doc.merge_table_cells_native(sec, para, ctrl, sr, sc, er, ec)
        .map_err(|e| (INVALID_PARAMS, format!("셀 병합 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn insert_table_row(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let ridx = u64_arg(args, "row")? as u16;
    let below = args.get("below").and_then(|v| v.as_bool()).unwrap_or(true);
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let (sec, para, ctrl) = resolve_table(&doc, table)?;
    doc.insert_table_row_native(sec, para, ctrl, ridx, below)
        .map_err(|e| (INVALID_PARAMS, format!("행 삽입 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn insert_table_column(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let cidx = u64_arg(args, "col")? as u16;
    let right = args.get("right").and_then(|v| v.as_bool()).unwrap_or(true);
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let (sec, para, ctrl) = resolve_table(&doc, table)?;
    doc.insert_table_column_native(sec, para, ctrl, cidx, right)
        .map_err(|e| (INVALID_PARAMS, format!("열 삽입 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn delete_table_row(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let ridx = u64_arg(args, "row")? as u16;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let (sec, para, ctrl) = resolve_table(&doc, table)?;
    doc.delete_table_row_native(sec, para, ctrl, ridx)
        .map_err(|e| (INVALID_PARAMS, format!("행 삭제 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn delete_table_column(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let cidx = u64_arg(args, "col")? as u16;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let (sec, para, ctrl) = resolve_table(&doc, table)?;
    doc.delete_table_column_native(sec, para, ctrl, cidx)
        .map_err(|e| (INVALID_PARAMS, format!("열 삭제 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn set_page_setup(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = args.get("section").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let mut props = serde_json::Map::new();
    // mm → HWPUNIT (1mm = 7200/25.4)
    let mm2hu = |mm: f64| (mm * 7200.0 / 25.4).round() as i64;
    for (arg, key) in [
        ("width_mm", "width"),
        ("height_mm", "height"),
        ("margin_left_mm", "marginLeft"),
        ("margin_right_mm", "marginRight"),
        ("margin_top_mm", "marginTop"),
        ("margin_bottom_mm", "marginBottom"),
        ("margin_header_mm", "marginHeader"),
        ("margin_footer_mm", "marginFooter"),
        ("margin_gutter_mm", "marginGutter"),
    ] {
        if let Some(mm) = args.get(arg).and_then(|v| v.as_f64()) {
            props.insert(key.to_string(), json!(mm2hu(mm)));
        }
    }
    if let Some(b) = args.get("landscape").and_then(|v| v.as_bool()) {
        props.insert("landscape".to_string(), json!(b));
    }
    if props.is_empty() {
        return Err((
            INVALID_PARAMS,
            "설정할 값(landscape/width_mm/height_mm/margin_*_mm) 하나 이상 필요".to_string(),
        ));
    }
    let props_json = Value::Object(props).to_string();
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    doc.set_page_def_native(section, &props_json)
        .map_err(|e| (INVALID_PARAMS, format!("페이지 설정 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn insert_footnote(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let char_offset = u64_arg(args, "char_offset")?;
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'text' 필요".to_string()))?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let raw = doc
        .insert_footnote_native(section, para, char_offset)
        .map_err(|e| (INVALID_PARAMS, format!("각주 삽입 실패: {e}")))?;
    let parsed: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
    let ctrl_idx = parsed
        .get("controlIdx")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    if !text.is_empty() {
        doc.insert_text_in_footnote_native(section, para, ctrl_idx, 0, 0, text)
            .map_err(|e| (INTERNAL_ERROR, format!("각주 내용 입력 실패: {e}")))?;
    }
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true, "footnote_number": parsed.get("footnoteNumber").cloned().unwrap_or(json!(null))}))
}

async fn add_bookmark(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let char_offset = u64_arg(args, "char_offset")?;
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'name' 필요".to_string()))?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    doc.add_bookmark_native(section, para, char_offset, name)
        .map_err(|e| (INVALID_PARAMS, format!("북마크 추가 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn list_bookmarks(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let raw = doc
        .get_bookmarks_native()
        .map_err(|e| (INTERNAL_ERROR, format!("북마크 조회 실패: {e}")))?;
    let bms: Value =
        serde_json::from_str(&raw).map_err(|e| (INTERNAL_ERROR, format!("북마크 파싱: {e}")))?;
    let count = bms.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({"bookmarks": bms, "count": count}))
}

async fn delete_bookmark(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = u64_arg(args, "section")?;
    let para = u64_arg(args, "para")?;
    let ctrl_idx = u64_arg(args, "ctrl_idx")?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    doc.delete_bookmark_native(section, para, ctrl_idx)
        .map_err(|e| (INVALID_PARAMS, format!("북마크 삭제 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn split_table_cell(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let cell_row = u64_arg(args, "row")? as u16;
    let cell_col = u64_arg(args, "col")? as u16;
    let n_rows = (u64_arg(args, "rows")? as u16).max(1);
    let m_cols = (u64_arg(args, "cols")? as u16).max(1);
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let (sec, para, ctrl) = resolve_table(&doc, table)?;
    doc.split_table_cell_into_native(sec, para, ctrl, cell_row, cell_col, n_rows, m_cols, true, false)
        .map_err(|e| (INVALID_PARAMS, format!("셀 분할 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn add_heading(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let level = u64_arg(args, "level")?.clamp(1, 6);
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'text' 필요".to_string()))?;
    let esc = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let html = format!("<h{level}>{esc}</h{level}>");
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let sections = doc.get_section_count() as usize;
    let (sec, para, off) = if sections == 0 {
        (0, 0, 0)
    } else {
        let ls = sections - 1;
        let pc = doc.get_paragraph_count_native(ls).unwrap_or(1).max(1);
        let lp = pc - 1;
        let co = doc.get_paragraph_length_native(ls, lp).unwrap_or(0);
        (ls, lp, co)
    };
    doc.paste_html_native(sec, para, off, &html)
        .map_err(|e| (INTERNAL_ERROR, format!("제목 삽입 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true, "level": level}))
}

async fn get_table_map(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    serde_json::from_str(&doc.get_table_map_native(table))
        .map_err(|e| (INTERNAL_ERROR, format!("표 맵 파싱: {e}")))
}

async fn find_cell_by_label(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'label' 필요".to_string()))?;
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("right");
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    serde_json::from_str(&doc.find_cell_by_label_native(table, label, direction))
        .map_err(|e| (INTERNAL_ERROR, format!("셀 조회 파싱: {e}")))
}

async fn list_styles(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let styles: Value = serde_json::from_str(&doc.list_styles_native())
        .map_err(|e| (INTERNAL_ERROR, format!("스타일 파싱: {e}")))?;
    let count = styles.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({"styles": styles, "count": count}))
}

async fn get_document_outline(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let outline: Value = serde_json::from_str(&doc.get_document_outline_native())
        .map_err(|e| (INTERNAL_ERROR, format!("개요 파싱: {e}")))?;
    let count = outline.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({"outline": outline, "count": count}))
}

async fn export_html(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let pages = doc.page_count();
    let mut html = String::new();
    for p in 0..pages {
        if let Ok(h) = doc.render_page_html_native(p) {
            html.push_str(&h);
            html.push('\n');
        }
    }
    Ok(json!({"html": html}))
}

async fn set_header_footer(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = args.get("section").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let apply_to = args.get("apply_to").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let header_text = args.get("header_text").and_then(|v| v.as_str());
    let footer_text = args.get("footer_text").and_then(|v| v.as_str());
    if header_text.is_none() && footer_text.is_none() {
        return Err((
            INVALID_PARAMS,
            "header_text 또는 footer_text 하나 이상 필요".to_string(),
        ));
    }
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    if let Some(t) = header_text {
        // 이미 있으면 create는 에러를 반환하므로 무시하고 텍스트 입력으로 진행.
        let _ = doc.create_header_footer_native(section, true, apply_to);
        doc.insert_text_in_header_footer_native(section, true, apply_to, 0, 0, t)
            .map_err(|e| (INVALID_PARAMS, format!("머리말 입력 실패: {e}")))?;
    }
    if let Some(t) = footer_text {
        let _ = doc.create_header_footer_native(section, false, apply_to);
        doc.insert_text_in_header_footer_native(section, false, apply_to, 0, 0, t)
            .map_err(|e| (INVALID_PARAMS, format!("꼬리말 입력 실패: {e}")))?;
    }
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

async fn set_page_number(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let section = args.get("section").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let apply_to = args.get("apply_to").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let is_header = args.get("location").and_then(|v| v.as_str()) == Some("header");
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let _ = doc.create_header_footer_native(section, is_header, apply_to);
    // 기존 머리말/꼬리말 텍스트 끝에 삽입한다. offset 0에 넣으면 기존 텍스트가 손상된다.
    let end_offset = doc
        .get_header_footer_native(section, is_header, apply_to)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|j| {
            j.get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().count())
        })
        .unwrap_or(0);
    // field_type 1 = 현재 쪽번호
    doc.insert_field_in_hf_native(section, is_header, apply_to, 0, end_offset, 1)
        .map_err(|e| (INVALID_PARAMS, format!("쪽번호 삽입 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true}))
}

/// 문서의 비어있지 않은 문단 텍스트 목록.
fn doc_paragraphs(doc: &crate::wasm_api::HwpDocument) -> Vec<String> {
    let mut out = Vec::new();
    let sections = doc.get_section_count() as usize;
    for s in 0..sections {
        let paras = doc.get_paragraph_count_native(s).unwrap_or(0);
        for p in 0..paras {
            let len = doc.get_paragraph_length_native(s, p).unwrap_or(0);
            if len > 0 {
                if let Ok(t) = doc.get_text_range_native(s, p, 0, len) {
                    if !t.trim().is_empty() {
                        out.push(t);
                    }
                }
            }
        }
    }
    out
}

/// 문단 시퀀스 LCS diff → (tag, text). tag: equal/removed(a에만)/added(b에만).
fn lcs_diff(a: &[String], b: &[String]) -> Vec<(&'static str, String)> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(("equal", a[i].clone()));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push(("removed", a[i].clone()));
            i += 1;
        } else {
            out.push(("added", b[j].clone()));
            j += 1;
        }
    }
    while i < n {
        out.push(("removed", a[i].clone()));
        i += 1;
    }
    while j < m {
        out.push(("added", b[j].clone()));
        j += 1;
    }
    out
}

async fn mail_merge(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let rows = args
        .get("rows")
        .and_then(|v| v.as_array())
        .ok_or((INVALID_PARAMS, "'rows' 배열 필요".to_string()))?;
    if rows.is_empty() {
        return Err((INVALID_PARAMS, "'rows'가 비어 있음".to_string()));
    }
    let name_prefix = args
        .get("name_prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("문서");
    // handle = 템플릿 문서
    let template_row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut out = Vec::new();
    for (i, rowv) in rows.iter().enumerate() {
        let fields = match rowv.as_object() {
            Some(o) => o,
            None => continue,
        };
        // 매 행마다 템플릿을 새로 로드해 치환.
        let mut doc = doc_ctx::load_doc(state, &template_row).await?;
        for (key, val) in fields {
            let value = match val {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            let placeholder = ["{{", key.as_str(), "}}"].concat();
            let _ = doc.replace_all_native(&placeholder, &value, false);
        }
        let bytes = doc
            .export_hwp_with_adapter()
            .map_err(|e| (INTERNAL_ERROR, format!("직렬화 실패: {e}")))?;
        let name = format!("{name_prefix}_{}", i + 1);
        let (_id, handle, _etag) =
            doc_ctx::create_stored_doc(state, user, &name, &bytes, "hwp").await?;
        out.push(json!({"handle": handle, "name": name}));
    }
    Ok(json!({"documents": out, "count": out.len()}))
}

async fn doc_diff(
    state: &AppState,
    user: Uuid,
    _sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let ha = args
        .get("handle_a")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'handle_a' 필요".to_string()))?;
    let hb = args
        .get("handle_b")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'handle_b' 필요".to_string()))?;
    let row_a = state
        .db
        .get_document_by_handle(user, ha)
        .await
        .map_err(|e| (INTERNAL_ERROR, e))?
        .ok_or((INVALID_PARAMS, "handle_a 문서 없음".to_string()))?;
    let row_b = state
        .db
        .get_document_by_handle(user, hb)
        .await
        .map_err(|e| (INTERNAL_ERROR, e))?
        .ok_or((INVALID_PARAMS, "handle_b 문서 없음".to_string()))?;
    let doc_a = doc_ctx::load_doc(state, &row_a).await?;
    let doc_b = doc_ctx::load_doc(state, &row_b).await?;
    let diff = lcs_diff(&doc_paragraphs(&doc_a), &doc_paragraphs(&doc_b));
    let (mut added, mut removed, mut equal) = (0usize, 0usize, 0usize);
    let mut items = Vec::with_capacity(diff.len());
    for (tag, text) in &diff {
        match *tag {
            "added" => added += 1,
            "removed" => removed += 1,
            _ => equal += 1,
        }
        items.push(json!({"tag": tag, "text": text}));
    }
    Ok(json!({"diff": items, "summary": {"added": added, "removed": removed, "equal": equal}}))
}

/// 콤마/단위 등을 제거하고 숫자만 파싱.
fn parse_num(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    if cleaned.is_empty() || cleaned == "-" || cleaned == "." {
        return None;
    }
    cleaned.parse::<f64>().ok()
}

/// HTML 특수문자 최소 이스케이프.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

async fn table_compute(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let table = u64_arg(args, "table")?;
    let column = u64_arg(args, "column")? as u16;
    let target_row = u64_arg(args, "target_row")? as u16;
    let op = args
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("sum");
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let (sec, para, ctrl) = resolve_table(&doc, table)?;
    // 표 셀 읽어 대상 열의 숫자 수집(대상 행 제외)
    let map: Value = serde_json::from_str(&doc.get_table_map_native(table))
        .map_err(|e| (INTERNAL_ERROR, format!("표 맵 파싱: {e}")))?;
    let mut vals: Vec<f64> = Vec::new();
    if let Some(cells) = map["cells"].as_array() {
        for c in cells {
            let cc = c.get("col").and_then(|v| v.as_u64());
            let cr = c.get("row").and_then(|v| v.as_u64());
            if cc == Some(column as u64) && cr != Some(target_row as u64) {
                if let Some(n) = c.get("text").and_then(|v| v.as_str()).and_then(parse_num) {
                    vals.push(n);
                }
            }
        }
    }
    let result = match op {
        "average" | "avg" => {
            if vals.is_empty() {
                0.0
            } else {
                vals.iter().sum::<f64>() / vals.len() as f64
            }
        }
        _ => vals.iter().sum::<f64>(),
    };
    let result_str = if (result.fract()).abs() < 1e-9 {
        format!("{}", result as i64)
    } else {
        format!("{result:.2}")
    };
    doc.set_cell_text_native(sec, para, ctrl, target_row, column, &result_str)
        .map_err(|e| (INVALID_PARAMS, format!("결과 셀 설정 실패: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true, "operation": op, "result": result, "count": vals.len(), "value": result_str}))
}

async fn create_comparison_table(
    state: &AppState,
    user: Uuid,
    _sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let ha = args
        .get("handle_a")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'handle_a' 필요".to_string()))?;
    let hb = args
        .get("handle_b")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'handle_b' 필요".to_string()))?;
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("신구대조표");
    let row_a = state
        .db
        .get_document_by_handle(user, ha)
        .await
        .map_err(|e| (INTERNAL_ERROR, e))?
        .ok_or((INVALID_PARAMS, "handle_a 문서 없음".to_string()))?;
    let row_b = state
        .db
        .get_document_by_handle(user, hb)
        .await
        .map_err(|e| (INTERNAL_ERROR, e))?
        .ok_or((INVALID_PARAMS, "handle_b 문서 없음".to_string()))?;
    let doc_a = doc_ctx::load_doc(state, &row_a).await?;
    let doc_b = doc_ctx::load_doc(state, &row_b).await?;
    let diff = lcs_diff(&doc_paragraphs(&doc_a), &doc_paragraphs(&doc_b));
    // 신구대조표 HTML: 구분 | 기존 | 변경
    let mut html = String::from(
        "<h1>신구대조표</h1><table><tr><td>구분</td><td>기존</td><td>변경</td></tr>",
    );
    for (tag, text) in &diff {
        let e = html_escape(text);
        match *tag {
            "removed" => html.push_str(&format!("<tr><td>삭제</td><td>{e}</td><td></td></tr>")),
            "added" => html.push_str(&format!("<tr><td>추가</td><td></td><td>{e}</td></tr>")),
            _ => {}
        }
    }
    html.push_str("</table>");
    // 새 문서 생성
    let mut doc = crate::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native()
        .map_err(|e| (INTERNAL_ERROR, format!("빈 문서 생성: {e}")))?;
    doc.paste_html_native(0, 0, 0, &html)
        .map_err(|e| (INTERNAL_ERROR, format!("표 삽입: {e}")))?;
    let bytes = doc
        .export_hwp_with_adapter()
        .map_err(|e| (INTERNAL_ERROR, format!("직렬화: {e}")))?;
    let (_id, handle, _etag) = doc_ctx::create_stored_doc(state, user, name, &bytes, "hwp").await?;
    let changes = diff.iter().filter(|(t, _)| *t != "equal").count();
    Ok(json!({"ok": true, "handle": handle, "name": name, "changes": changes}))
}

/// 텍스트에서 콤마 없는 4자리 이상 숫자 런을 찾는다(천단위 구분 누락 후보).
fn uncommaed_amounts(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i - start >= 4 {
                out.push(chars[start..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    out
}

async fn official_lint(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let paras = doc_paragraphs(&doc);
    let mut violations: Vec<Value> = Vec::new();

    // 규칙 1: 끝 표시 — 마지막 내용 문단이 "끝."으로 끝나야 함.
    if let Some(last) = paras.iter().rev().find(|p| !p.trim().is_empty()) {
        if !last.trim_end().ends_with("끝.") {
            violations.push(json!({
                "rule": "end-marker",
                "message": "마지막 문단이 '끝.'으로 끝나지 않습니다.",
                "text": last
            }));
        }
    }

    // 규칙 2: 금액 천단위 콤마 — 4자리 이상 숫자에 콤마가 없으면 지적.
    for (i, p) in paras.iter().enumerate() {
        for amount in uncommaed_amounts(p) {
            violations.push(json!({
                "rule": "amount-notation",
                "message": format!("숫자 '{amount}'에 천단위 구분(콤마)이 없습니다."),
                "paragraph": i,
                "text": p
            }));
        }
    }

    let ok = violations.is_empty();
    // 점수: 문단 수 대비 위반 비율(대략), 0~1.
    let denom = paras.len().max(1) as f64;
    let score = (1.0 - (violations.len() as f64 / denom)).max(0.0);
    Ok(json!({
        "ok": ok,
        "score": (score * 100.0).round() / 100.0,
        "violation_count": violations.len(),
        "violations": violations
    }))
}

/// HTML로 새 문서를 만들어 저장하고 handle을 반환한다(생성기 공통).
async fn create_doc_from_html(
    state: &AppState,
    user: Uuid,
    name: &str,
    html: &str,
) -> Result<String, (i64, String)> {
    let mut doc = crate::wasm_api::HwpDocument::create_empty();
    doc.create_blank_document_native()
        .map_err(|e| (INTERNAL_ERROR, format!("빈 문서 생성: {e}")))?;
    if !html.trim().is_empty() {
        doc.paste_html_native(0, 0, 0, html)
            .map_err(|e| (INTERNAL_ERROR, format!("본문 삽입: {e}")))?;
    }
    let bytes = doc
        .export_hwp_with_adapter()
        .map_err(|e| (INTERNAL_ERROR, format!("직렬화: {e}")))?;
    let (_id, handle, _etag) = doc_ctx::create_stored_doc(state, user, name, &bytes, "hwp").await?;
    Ok(handle)
}

/// 조직도 계층의 최대 깊이.
fn org_depth(node: &Value) -> usize {
    let child_max = node
        .get("children")
        .and_then(|v| v.as_array())
        .map(|cs| cs.iter().map(org_depth).max().unwrap_or(0))
        .unwrap_or(0);
    1 + child_max
}
/// 노드를 (깊이=열) 표 행으로 재귀 전개.
fn org_rows(node: &Value, depth: usize, max_depth: usize, rows: &mut Vec<String>) {
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let mut cells = String::new();
    for c in 0..max_depth {
        if c == depth {
            cells.push_str(&format!("<td>{}</td>", html_escape(name)));
        } else {
            cells.push_str("<td></td>");
        }
    }
    rows.push(format!("<tr>{cells}</tr>"));
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for ch in children {
            org_rows(ch, depth + 1, max_depth, rows);
        }
    }
}

async fn build_organization_chart(
    state: &AppState,
    user: Uuid,
    _sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let h = args
        .get("hierarchy")
        .ok_or((INVALID_PARAMS, "'hierarchy' 필요".to_string()))?;
    let roots: Vec<Value> = match h {
        Value::Array(a) => a.clone(),
        Value::Object(_) => vec![h.clone()],
        _ => return Err((INVALID_PARAMS, "hierarchy는 객체 또는 배열".to_string())),
    };
    let max_depth = roots.iter().map(org_depth).max().unwrap_or(1).max(1);
    let mut rows = Vec::new();
    for r in &roots {
        org_rows(r, 0, max_depth, &mut rows);
    }
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("조직도");
    let html = format!("<h1>{}</h1><table>{}</table>", html_escape(name), rows.join(""));
    let handle = create_doc_from_html(state, user, name, &html).await?;
    Ok(json!({"ok": true, "handle": handle, "name": name, "nodes": rows.len()}))
}

async fn build_image_grid(
    state: &AppState,
    user: Uuid,
    _sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let images = args
        .get("images")
        .and_then(|v| v.as_array())
        .ok_or((INVALID_PARAMS, "'images' 배열 필요".to_string()))?;
    if images.is_empty() {
        return Err((INVALID_PARAMS, "images가 비어 있음".to_string()));
    }
    let columns = args.get("columns").and_then(|v| v.as_u64()).unwrap_or(2).max(1) as usize;
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("사진그리드");
    let mut html = String::from("<table>");
    for chunk in images.chunks(columns) {
        html.push_str("<tr>");
        for img in chunk {
            let (src, caption) = match img {
                Value::String(s) => (s.as_str(), ""),
                Value::Object(_) => (
                    img.get("data").and_then(|v| v.as_str()).unwrap_or(""),
                    img.get("caption").and_then(|v| v.as_str()).unwrap_or(""),
                ),
                _ => ("", ""),
            };
            // src(data URI)는 이스케이프하지 않음. caption만 이스케이프.
            let cap = if caption.is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>", html_escape(caption))
            };
            html.push_str(&format!("<td><img src=\"{src}\"/>{cap}</td>"));
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    let handle = create_doc_from_html(state, user, name, &html).await?;
    Ok(json!({"ok": true, "handle": handle, "name": name, "count": images.len()}))
}

async fn build_meeting_nameplates(
    state: &AppState,
    user: Uuid,
    _sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let people = args
        .get("participants")
        .and_then(|v| v.as_array())
        .ok_or((INVALID_PARAMS, "'participants' 배열 필요".to_string()))?;
    if people.is_empty() {
        return Err((INVALID_PARAMS, "participants가 비어 있음".to_string()));
    }
    let columns = args.get("columns").and_then(|v| v.as_u64()).unwrap_or(2).max(1) as usize;
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("명패");
    let mut html = String::from("<table>");
    for chunk in people.chunks(columns) {
        html.push_str("<tr>");
        for p in chunk {
            let pname = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let title = p.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let title_html = if title.is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>", html_escape(title))
            };
            html.push_str(&format!(
                "<td><h2>{}</h2>{}</td>",
                html_escape(pname),
                title_html
            ));
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    let handle = create_doc_from_html(state, user, name, &html).await?;
    Ok(json!({"ok": true, "handle": handle, "name": name, "count": people.len()}))
}

async fn repair_hwpx(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    // 원본을 파싱(load_doc)한 뒤 재직렬화하면 구조가 정규화된다.
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let bytes = doc
        .export_hwp_with_adapter()
        .map_err(|e| (INTERNAL_ERROR, format!("재직렬화 실패: {e}")))?;
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}_복구", row.name));
    let (_id, handle, _etag) =
        doc_ctx::create_stored_doc(state, user, &name, &bytes, "hwp").await?;
    Ok(json!({"ok": true, "handle": handle, "name": name, "bytes": bytes.len()}))
}

async fn render_preview(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let page = args.get("page").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("svg");
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let page_count = doc.page_count();
    if page >= page_count {
        return Err((
            INVALID_PARAMS,
            format!("페이지 {page} 없음(총 {page_count}쪽)"),
        ));
    }
    match format {
        // SVG는 <text> 기반 → 클라이언트(브라우저/아티팩트)가 한글 렌더. 인라인 반환.
        "svg" => {
            let svg = doc
                .render_page_svg_native(page)
                .map_err(|e| (INTERNAL_ERROR, format!("SVG 렌더 실패: {e}")))?;
            Ok(json!({"format": "svg", "page": page, "page_count": page_count, "svg": svg}))
        }
        // PNG는 서버 래스터(resvg/tiny-skia). LLM이 이미지로 직접 보고 분석한다(PDF/이미지 수준).
        // Claude Vision 한도(긴 변 1568px)에 맞춰 렌더. content 배열로 반환 → 그대로 통과.
        "png" => {
            #[cfg(feature = "svg-raster")]
            {
                let svg = doc
                    .render_page_svg_native(page)
                    .map_err(|e| (INTERNAL_ERROR, format!("SVG 렌더 실패: {e}")))?;
                let png = crate::renderer::pdf::svg_to_png(&svg, 1568)
                    .map_err(|e| (INTERNAL_ERROR, format!("PNG 래스터 실패: {e}")))?;
                let caption = format!(
                    "'{}' 페이지 {}/{} (PNG, {}바이트)",
                    row.name,
                    page + 1,
                    page_count,
                    png.len()
                );
                Ok(rpc::tool_image_result(&png, &caption))
            }
            #[cfg(not(feature = "svg-raster"))]
            {
                Err((INVALID_PARAMS, "png 렌더는 svg-raster 빌드가 필요합니다".to_string()))
            }
        }
        // PDF는 서버 렌더(svg2pdf, 텍스트→아웃라인에 fonts-nanum 사용). 저장 후 다운로드 URL 반환.
        "pdf" => {
            let bytes = doc
                .render_page_pdf_native(page)
                .map_err(|e| (INTERNAL_ERROR, format!("PDF 렌더 실패: {e}")))?;
            let key = format!("previews/{}/{}.pdf", user, Uuid::new_v4());
            state
                .store
                .put(&key, &bytes)
                .await
                .map_err(|e| (INTERNAL_ERROR, format!("저장 실패: {e}")))?;
            let filename = format!("{}_p{}.pdf", row.name, page);
            let token = state.downloads.issue(&key, &filename, 86_400);
            let base = state.public_base_url.trim_end_matches('/');
            let url = format!("{base}/download/{token}");
            Ok(json!({"format": "pdf", "page": page, "page_count": page_count, "download_url": url, "bytes": bytes.len()}))
        }
        other => Err((INVALID_PARAMS, format!("지원 형식: svg/pdf (받음: {other})"))),
    }
}

async fn import_document(state: &AppState, user: Uuid, args: &Value) -> Result<Value, (i64, String)> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'name' 필요".to_string()))?;
    // 빈 placeholder 없이, 업로드 대상 키를 미리 만들고 presigned PUT 발급 + documents 행 생성(빈 etag)
    let doc_id = Uuid::new_v4();
    let key = doc_ctx::storage_key(user, doc_id);
    let handle = doc_ctx::new_handle(name);
    state
        .db
        .create_document(user, &handle, name, &key, "hwp")
        .await
        .map_err(|e| (INTERNAL_ERROR, format!("등록 실패: {e}")))?;
    // 앱 프록시 업로드 링크(외부에서 PUT 가능, minio 비노출). presigned는 내부 호스트라 사용 불가.
    let token = state.uploads.issue(&key, 3600);
    let base = state.public_base_url.trim_end_matches('/');
    let url = format!("{base}/upload/{token}");
    Ok(json!({"handle": handle, "upload_url": url, "expires_secs": 3600,
              "note":"이 URL로 HWP를 PUT 업로드한 뒤 handle로 사용하세요."}))
}

async fn export_document(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("hwp");
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let bytes = match format {
        "hwpx" => doc
            .export_hwpx_native()
            .map_err(|e| (INTERNAL_ERROR, format!("HWPX 직렬화: {e}")))?,
        "hwp" => doc
            .export_hwp_with_adapter()
            .map_err(|e| (INTERNAL_ERROR, format!("HWP 직렬화: {e}")))?,
        other => return Err((INVALID_PARAMS, format!("지원하지 않는 포맷: {other}"))),
    };
    // Axis 3: 나가기 전에 출력 바이트를 구조 검증(레코드 경계·필수 스트림·압축 무결).
    // "열리는 파일"을 기계적으로 보증 — 렌더/파서가 관대해 놓치던 결함을 미리 잡는다.
    let report = super::validate::validate(&bytes);
    let etag = state
        .store
        .put(&row.storage_key, &bytes)
        .await
        .map_err(|e| (INTERNAL_ERROR, format!("저장: {e}")))?;
    let _ = state.db.update_document_etag(row.id, &etag).await;
    // 앱 프록시 다운로드 링크(외부에서 열림, minio 비노출). presigned는 내부 호스트라 사용 불가.
    // 사용자가 나중에 열어볼 수 있도록 24시간 유효(인메모리라 재배포 시에는 초기화됨).
    let filename = format!("{}.{}", row.name, format);
    let token = state.downloads.issue(&row.storage_key, &filename, 86_400);
    let base = state.public_base_url.trim_end_matches('/');
    let url = format!("{base}/download/{token}");
    Ok(json!({
        "ok": true, "format": format, "download_url": url, "expires_secs": 86_400,
        "valid": report.valid, "validation": report.to_json()
    }))
}

/// 문서를 지정 포맷으로 직렬화해 구조 검증 결과만 반환한다(저장/업로드 없음).
async fn validate_document(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("hwp");
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    let bytes = match format {
        "hwpx" => doc
            .export_hwpx_native()
            .map_err(|e| (INTERNAL_ERROR, format!("HWPX 직렬화: {e}")))?,
        "hwp" => doc
            .export_hwp_with_adapter()
            .map_err(|e| (INTERNAL_ERROR, format!("HWP 직렬화: {e}")))?,
        other => return Err((INVALID_PARAMS, format!("지원하지 않는 포맷: {other}"))),
    };
    let report = super::validate::validate(&bytes);
    Ok(json!({"format": format, "bytes": bytes.len(), "valid": report.valid, "validation": report.to_json()}))
}

async fn list_fields(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let raw = doc.get_field_list_json();
    let fields: Value =
        serde_json::from_str(&raw).map_err(|e| (INTERNAL_ERROR, format!("필드 파싱: {e}")))?;
    let count = fields.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({"fields": fields, "count": count}))
}

async fn get_field_value(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'name' 필요".to_string()))?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let doc = doc_ctx::load_doc(state, &row).await?;
    let value = doc
        .get_field_value_by_name(name)
        .map_err(|e| (INVALID_PARAMS, format!("필드 조회: {e}")))?;
    Ok(json!({"name": name, "value": value}))
}

async fn set_field_value(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'name' 필요".to_string()))?;
    let value = args
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "'value' 필요".to_string()))?;
    let row = doc_ctx::resolve_row(state, user, sid, args).await?;
    let mut doc = doc_ctx::load_doc(state, &row).await?;
    doc.set_field_value_by_name(name, value)
        .map_err(|e| (INVALID_PARAMS, format!("필드 설정: {e}")))?;
    doc_ctx::save_doc(state, &row, &mut doc).await?;
    Ok(json!({"ok": true, "name": name, "value": value}))
}
