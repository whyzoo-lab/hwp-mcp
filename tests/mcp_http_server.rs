//! Phase 2 HTTP MCP 서버 통합 테스트. feature = "mcp-http".
#![cfg(feature = "mcp-http")]

use rhwp::mcp::http::config::Config;
use rhwp::mcp::http::server::{build_router, AppState};

fn dev_config() -> Config {
    Config {
        database_url: "postgres://rhwp:rhwp@localhost:5433/rhwp".into(),
        s3_endpoint: "http://localhost:9002".into(),
        s3_region: "us-east-1".into(),
        s3_bucket: "rhwp-docs".into(),
        s3_access_key: "minioadmin".into(),
        s3_secret_key: "minioadmin".into(),
        s3_use_path_style: true,
        cookie_secure: false,
        public_base_url: "http://localhost:8300".into(),
        auth_required: true,
    }
}

/// dev 스택 필요. 미기동 시 skip.
async fn test_state() -> Option<AppState> {
    AppState::connect(&dev_config()).await.ok()
}

#[tokio::test]
async fn healthz_ok() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip: dev 스택 미기동");
            return;
        }
    };
    let app = build_router(state);
    let resp = axum_test_get(app, "/healthz").await;
    assert_eq!(resp.0, 200);
}

#[tokio::test]
async fn mcp_initialize_requires_auth() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let app = build_router(state);
    // 토큰 없이 initialize → 401
    let body =
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}).to_string();
    let resp = axum_test_post(app, "/mcp", &body, None).await;
    assert_eq!(resp.0, 401);
}

#[tokio::test]
async fn mcp_initialize_with_valid_token() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    // 사용자+토큰 준비(Phase 1 API 사용)
    let db = &state.db;
    db.migrate().await.unwrap();
    let uname = format!("http_{}", uuid::Uuid::new_v4());
    let uid = db.create_user(&uname, "pw", false).await.unwrap();
    let token = rhwp::mcp::http::auth::generate_token();
    db.issue_token(uid, &rhwp::mcp::http::auth::hash_secret(&token).unwrap())
        .await
        .unwrap();

    let app = build_router(state);
    let body =
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}).to_string();
    let resp = axum_test_post(app, "/mcp", &body, Some(&token)).await;
    assert_eq!(resp.0, 200);
    let v: serde_json::Value = serde_json::from_str(&resp.1).unwrap();
    assert!(v["result"]["serverInfo"]["name"].is_string());
}

/// 인증 tools/call 호출 후 content[0].text의 JSON을 파싱.
async fn call_tool(
    app: axum::Router,
    token: &str,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let body = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":name,"arguments":args}})
    .to_string();
    let (code, text) = axum_test_post(app, "/mcp", &body, Some(token)).await;
    assert_eq!(code, 200, "http {code}: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(v.get("error").is_none(), "rpc error: {v}");
    let inner = v["result"]["content"][0]["text"].as_str().expect("text");
    serde_json::from_str(inner).unwrap()
}

async fn make_user_token(state: &AppState) -> String {
    state.db.migrate().await.unwrap();
    let uname = format!("http_{}", uuid::Uuid::new_v4());
    let uid = state.db.create_user(&uname, "pw", false).await.unwrap();
    let token = rhwp::mcp::http::auth::generate_token();
    state
        .db
        .issue_token(uid, &rhwp::mcp::http::auth::hash_secret(&token).unwrap())
        .await
        .unwrap();
    token
}

#[tokio::test]
async fn create_and_list_documents() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"계약서","html":"<p>내용 KEEP_A</p>"}),
    )
    .await;
    let handle = created["handle"].as_str().unwrap().to_string();
    assert!(!handle.is_empty());

    let listed = call_tool(app.clone(), &token, "list_documents", serde_json::json!({})).await;
    let docs = listed["documents"].as_array().unwrap();
    assert!(docs
        .iter()
        .any(|d| d["handle"] == serde_json::json!(handle)));

    let info = call_tool(
        app,
        &token,
        "document_info",
        serde_json::json!({"handle": handle}),
    )
    .await;
    assert_eq!(info["name"], serde_json::json!("계약서"));
}

#[tokio::test]
async fn edit_roundtrip_replace_and_read() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"문서","html":"<p>이전문구 유지</p>"}),
    )
    .await;
    let handle = created["handle"].as_str().unwrap().to_string();

    let r = call_tool(
        app.clone(),
        &token,
        "replace_text",
        serde_json::json!({"handle": handle, "query":"이전문구","replacement":"새문구"}),
    )
    .await;
    assert_eq!(r["ok"], serde_json::json!(true));

    // 세션 현재문서로 read(핸들 생략) — 방금 replace가 세션 현재문서를 설정함
    let read = call_tool(app, &token, "read_document", serde_json::json!({})).await;
    let text = read["text"].as_str().unwrap();
    assert!(text.contains("새문구"), "저장본 반영: {text}");
    assert!(!text.contains("이전문구"));
}

#[tokio::test]
async fn export_returns_download_url_and_reloads() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"문서","html":"<p>KEEP_EXPORT_9</p>"}),
    )
    .await;
    let handle = created["handle"].as_str().unwrap().to_string();

    let out = call_tool(
        app.clone(),
        &token,
        "export_document",
        serde_json::json!({"handle": handle, "format":"hwp"}),
    )
    .await;
    let url = out["download_url"].as_str().unwrap().to_string();
    assert!(url.contains("/download/"), "앱 프록시 링크: {url}");

    // 앱 프록시 다운로드로 실제 바이트 수신 후 재로드 검증
    let bytes = download_via_app(app, &url).await;
    let doc = rhwp::wasm_api::HwpDocument::from_bytes(&bytes).expect("재로드");
    assert!(doc.get_section_count() >= 1);
}

/// 종단 e2e: 외부 도구가 준 HTML로 문서 생성 → 편집 → export → 다운로드 재로드.
#[tokio::test]
async fn e2e_html_to_hwp_over_http() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    // 외부 도구가 준 HTML로 문서 생성 → 편집 → export → 다운로드 재로드
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"보고서","html":"<h1>제목</h1><p>본문 KEEP_E2E_777</p>"}),
    )
    .await;
    let handle = created["handle"].as_str().unwrap().to_string();
    call_tool(
        app.clone(),
        &token,
        "replace_text",
        serde_json::json!({"handle": handle, "query":"제목","replacement":"최종 제목"}),
    )
    .await;
    let out = call_tool(
        app.clone(),
        &token,
        "export_document",
        serde_json::json!({"handle": handle, "format":"hwp"}),
    )
    .await;
    let url = out["download_url"].as_str().unwrap().to_string();
    let bytes = download_via_app(app.clone(), &url).await;

    // 다른 세션에서 재로드 후 본문 보존 확인(read via 새 create는 불가하므로 직접 파싱)
    let doc = rhwp::wasm_api::HwpDocument::from_bytes(&bytes).expect("재로드");
    assert!(doc.get_section_count() >= 1);
    // 텍스트 보존: read_document로 확인
    let read = call_tool(app, &token, "read_document", serde_json::json!({"handle": handle})).await;
    let text = read["text"].as_str().unwrap();
    assert!(text.contains("KEEP_E2E_777"), "본문 보존: {text}");
    assert!(text.contains("최종 제목"));
}

/// 회귀(#1+#2+#3): 과거 렌더링 패닉(rendering.rs:1979)을 일으키던 시험지형 HTML로
/// create → export → 다운로드가 502/에러 없이 완주하고 HWP(CFB)가 나온다.
#[tokio::test]
async fn table_exam_html_creates_and_downloads_over_http() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    let html = "<h1>자격시험</h1><p>다음 물음에 답하시오.</p><table><tr><td>문항</td><td>정답</td></tr><tr><td>1</td><td>②</td></tr></table>";
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"시험지","html": html}),
    )
    .await;
    let handle = created["handle"]
        .as_str()
        .expect("패닉 없이 생성되어 handle 반환")
        .to_string();
    let out = call_tool(
        app.clone(),
        &token,
        "export_document",
        serde_json::json!({"handle": handle, "format":"hwp"}),
    )
    .await;
    let url = out["download_url"].as_str().unwrap().to_string();
    let bytes = download_via_app(app, &url).await;
    assert!(
        bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]),
        "HWP(CFB) 매직으로 시작해야 함"
    );
}

/// 업로드-편집 경로(#import): import_document → 앱 프록시 PUT 업로드 → handle로 read.
#[tokio::test]
async fn import_upload_and_read_over_http() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    // 올릴 HWP 바이트: create+export로 만든 뒤 다운로드
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"orig","html":"<p>UPLOAD_SEED_42</p>"}),
    )
    .await;
    let h0 = created["handle"].as_str().unwrap().to_string();
    let out0 = call_tool(
        app.clone(),
        &token,
        "export_document",
        serde_json::json!({"handle": h0, "format":"hwp"}),
    )
    .await;
    let seed = download_via_app(app.clone(), out0["download_url"].as_str().unwrap()).await;

    // import_document → 앱 프록시 upload_url로 PUT
    let imp = call_tool(app.clone(), &token, "import_document", serde_json::json!({"name":"업로드본"})).await;
    let handle = imp["handle"].as_str().unwrap().to_string();
    let upload_url = imp["upload_url"].as_str().unwrap().to_string();
    assert!(upload_url.contains("/upload/"), "앱 프록시 업로드 링크: {upload_url}");
    let up_path = &upload_url[upload_url.find("/upload/").unwrap()..];
    let put_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(up_path)
                .body(Body::from(seed))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_resp.status().as_u16(), 200, "업로드 PUT 200");

    // 업로드본을 handle로 read → 본문 보존
    let read = call_tool(app, &token, "read_document", serde_json::json!({"handle": handle})).await;
    assert!(
        read["text"].as_str().unwrap().contains("UPLOAD_SEED_42"),
        "업로드본 재로드 본문 보존: {}",
        read["text"]
    );
}

/// read_document(format=markdown)가 표 셀 내용을 포함한다(plain은 문단 텍스트만).
#[tokio::test]
async fn read_document_markdown_includes_table() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"표문서","html":"<p>표앞</p><table><tr><td>문항</td><td>정답</td></tr><tr><td>1</td><td>②</td></tr></table>"}),
    )
    .await;
    let handle = created["handle"].as_str().unwrap().to_string();

    let md = call_tool(
        app.clone(),
        &token,
        "read_document",
        serde_json::json!({"handle": handle, "format":"markdown"}),
    )
    .await;
    let text = md["text"].as_str().unwrap();
    assert!(
        text.contains("문항") && text.contains("정답") && text.contains("②"),
        "markdown에 표 셀 포함: {text}"
    );
    assert!(text.contains('|'), "markdown 표 파이프 포함: {text}");

    // plain은 표 셀을 포함하지 않음(문단 텍스트만)
    let plain = call_tool(
        app,
        &token,
        "read_document",
        serde_json::json!({"handle": handle, "format":"plain"}),
    )
    .await;
    assert!(plain["text"].as_str().unwrap().contains("표앞"));
}

/// P1: format_text 도구가 search로 찾은 범위에 굵게/색/크기를 적용하고 export에 보존.
#[tokio::test]
async fn format_text_tool_applies_over_http() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"fmt","html":"<p>FORMATTARGET 텍스트</p>"}),
    )
    .await;
    let handle = created["handle"].as_str().unwrap().to_string();
    let found = call_tool(
        app.clone(),
        &token,
        "search",
        serde_json::json!({"handle": handle, "query":"FORMATTARGET"}),
    )
    .await;
    let m = &found["matches"][0];
    let sec = m["section"].as_u64().unwrap();
    let para = m["para"].as_u64().unwrap();
    let start = m["char_offset"].as_u64().unwrap();
    let len = m["length"].as_u64().unwrap();
    let fmt = call_tool(
        app.clone(),
        &token,
        "format_text",
        serde_json::json!({"handle": handle, "section": sec, "para": para, "start": start, "end": start+len, "bold": true, "color":"#ff0000", "font_size_pt": 16}),
    )
    .await;
    assert_eq!(fmt["ok"], serde_json::json!(true), "format_text ok");
    // 문단 가운데 정렬도
    let pf = call_tool(
        app.clone(),
        &token,
        "set_paragraph_format",
        serde_json::json!({"handle": handle, "section": sec, "para": para, "align":"center"}),
    )
    .await;
    assert_eq!(pf["ok"], serde_json::json!(true), "set_paragraph_format ok");

    let out = call_tool(
        app.clone(),
        &token,
        "export_document",
        serde_json::json!({"handle": handle, "format":"hwp"}),
    )
    .await;
    let bytes = download_via_app(app, out["download_url"].as_str().unwrap()).await;
    let ir = rhwp::parser::parse_document(&bytes).expect("reparse");
    assert!(
        ir.doc_info.char_shapes.iter().any(|c| c.bold && c.text_color == 0x0000FF),
        "굵게+빨강 적용 보존"
    );
    assert!(
        ir.doc_info.char_shapes.iter().any(|c| c.base_size == 1600),
        "16pt 적용 보존"
    );
    assert!(
        ir.doc_info.para_shapes.iter().any(|p| matches!(p.alignment, rhwp::model::style::Alignment::Center)),
        "가운데 정렬 보존"
    );
}

/// P1b: 표 편집 도구(list_tables/set_table_cell_text/insert_table_row) e2e.
#[tokio::test]
async fn table_edit_tools_over_http() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let token = make_user_token(&state).await;
    let app = build_router(state.clone());
    let created = call_tool(
        app.clone(),
        &token,
        "create_document",
        serde_json::json!({"name":"tbl","html":"<table><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></table>"}),
    )
    .await;
    let handle = created["handle"].as_str().unwrap().to_string();

    let lt = call_tool(app.clone(), &token, "list_tables", serde_json::json!({"handle": handle})).await;
    assert_eq!(lt["count"], serde_json::json!(1), "표 1개");
    assert_eq!(lt["tables"][0]["rows"], serde_json::json!(2));

    let sc = call_tool(
        app.clone(),
        &token,
        "set_table_cell_text",
        serde_json::json!({"handle": handle, "table": 0, "row": 1, "col": 1, "text":"셀변경"}),
    )
    .await;
    assert_eq!(sc["ok"], serde_json::json!(true));

    let ir_row = call_tool(
        app.clone(),
        &token,
        "insert_table_row",
        serde_json::json!({"handle": handle, "table": 0, "row": 0, "below": true}),
    )
    .await;
    assert_eq!(ir_row["ok"], serde_json::json!(true));

    // 행 3개로 늘었는지
    let lt2 = call_tool(app.clone(), &token, "list_tables", serde_json::json!({"handle": handle})).await;
    assert_eq!(lt2["tables"][0]["rows"], serde_json::json!(3), "행 3");

    // export → 재파싱하여 셀(1,1) 텍스트 확인
    let out = call_tool(
        app.clone(),
        &token,
        "export_document",
        serde_json::json!({"handle": handle, "format":"hwp"}),
    )
    .await;
    let bytes = download_via_app(app, out["download_url"].as_str().unwrap()).await;
    let doc = rhwp::parser::parse_document(&bytes).expect("reparse");
    let mut found = None;
    for p in &doc.sections[0].paragraphs {
        for c in &p.controls {
            if let rhwp::model::control::Control::Table(tb) = c {
                for cell in &tb.cells {
                    if cell.row == 1 && cell.col == 1 {
                        found = Some(cell.paragraphs.iter().map(|pp| pp.text.clone()).collect::<String>());
                    }
                }
            }
        }
    }
    assert_eq!(found.as_deref(), Some("셀변경"), "셀(1,1) 교체 보존");
}

/// 사용자 격리: 다른 사용자 토큰으로 남의 handle 접근 시 실패.
#[tokio::test]
async fn cross_user_isolation_over_http() {
    let state = match test_state().await {
        Some(s) => s,
        None => {
            eprintln!("skip");
            return;
        }
    };
    let t1 = make_user_token(&state).await;
    let t2 = make_user_token(&state).await;
    let app = build_router(state.clone());
    let created = call_tool(
        app.clone(),
        &t1,
        "create_document",
        serde_json::json!({"name":"비밀","html":"<p>비밀내용</p>"}),
    )
    .await;
    let handle = created["handle"].as_str().unwrap().to_string();
    // t2가 t1의 handle로 read → rpc error(문서 없음)
    let body = serde_json::json!({"jsonrpc":"2.0","id":9,"method":"tools/call",
        "params":{"name":"read_document","arguments":{"handle": handle}}})
    .to_string();
    let (code, text) = axum_test_post(app, "/mcp", &body, Some(&t2)).await;
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(v.get("error").is_some(), "타 사용자 접근은 에러여야: {v}");
}

// --- axum 0.8 in-process 테스트 헬퍼: tower::ServiceExt::oneshot 사용 ---
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn axum_test_get(app: axum::Router, path: &str) -> (u16, String) {
    let req = Request::builder().uri(path).body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let code = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (code, String::from_utf8_lossy(&bytes).to_string())
}

async fn axum_test_post(
    app: axum::Router,
    path: &str,
    body: &str,
    bearer: Option<&str>,
) -> (u16, String) {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let req = b.body(Body::from(body.to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let code = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let _ = StatusCode::OK;
    (code, String::from_utf8_lossy(&bytes).to_string())
}

/// 앱 프록시 다운로드 링크(`.../download/{token}`)에서 경로를 뽑아 /download 라우트로
/// raw 바이트를 받는다(토큰이 인증, 별도 헤더 불필요).
async fn download_via_app(app: axum::Router, download_url: &str) -> Vec<u8> {
    let idx = download_url
        .find("/download/")
        .expect("앱 프록시 다운로드 링크여야 함");
    let path = &download_url[idx..];
    let req = Request::builder().uri(path).body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200, "download 라우트 200");
    axum::body::to_bytes(resp.into_body(), 8 << 20)
        .await
        .unwrap()
        .to_vec()
}
