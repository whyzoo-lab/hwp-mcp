//! 문서 리소스 컨텍스트: 핸들 생성, 스토리지 키, load→edit→save.
use uuid::Uuid;

use super::db::DocumentRow;
use super::rpc::{INTERNAL_ERROR, INVALID_PARAMS};
use super::server::AppState;

/// 사람이 읽기 쉬운 핸들: <이름 슬러그>-<짧은 랜덤>. 전역 UNIQUE(스키마) 보장 위해 랜덤 접미.
pub fn new_handle(name: &str) -> String {
    let base: String = name
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(20)
        .collect();
    let base = if base.is_empty() {
        "doc".to_string()
    } else {
        base
    };
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{base}-{}", &suffix[..6])
}

pub fn storage_key(user: Uuid, doc_id: Uuid) -> String {
    format!("users/{user}/{doc_id}")
}

/// 새 문서 바이트를 저장하고 documents 행을 만든다. 반환: (doc_id, handle, etag).
pub async fn create_stored_doc(
    state: &AppState,
    user: Uuid,
    name: &str,
    bytes: &[u8],
    format: &str,
) -> Result<(Uuid, String, String), (i64, String)> {
    let doc_id = Uuid::new_v4();
    let key = storage_key(user, doc_id);
    let etag = state
        .store
        .put(&key, bytes)
        .await
        .map_err(|e| (INTERNAL_ERROR, format!("저장 실패: {e}")))?;
    let handle = new_handle(name);
    // create_document가 내부에서 자체 Uuid를 생성해 반환하므로, 그 반환값을
    // 유일한 권위 있는 doc_id로 사용한다(로컬 doc_id는 스토리지 키 계산에만 쓰인다).
    let returned_id = state
        .db
        .create_document(user, &handle, name, &key, format)
        .await
        .map_err(|e| (INTERNAL_ERROR, format!("문서 등록 실패: {e}")))?;
    // 초기 etag 기록
    let _ = state.db.update_document_etag(returned_id, &etag).await;
    Ok((returned_id, handle, etag))
}

/// 핸들 또는 세션 현재문서로 doc_id를 해석(소유권 검증 포함).
pub async fn resolve_row(
    state: &AppState,
    user: Uuid,
    sid: &str,
    args: &serde_json::Value,
) -> Result<DocumentRow, (i64, String)> {
    if let Some(h) = args.get("handle").and_then(|v| v.as_str()) {
        let row = state
            .db
            .get_document_by_handle(user, h)
            .await
            .map_err(|e| (INTERNAL_ERROR, e))?
            .ok_or((INVALID_PARAMS, format!("문서 없음: {h}")))?;
        state.sessions.set_current_doc(sid, row.id);
        return Ok(row);
    }
    // 세션 현재문서
    let cur = state.sessions.current_doc(sid).ok_or((
        INVALID_PARAMS,
        "열린 문서가 없습니다. handle 인자를 지정하거나(handle 지정 시 자동으로 현재 문서가 됩니다) create_document로 새 문서를 만드세요."
            .to_string(),
    ))?;
    // 현재문서를 user 스코프로 재확인
    let rows = state
        .db
        .list_documents(user)
        .await
        .map_err(|e| (INTERNAL_ERROR, e))?;
    rows.into_iter()
        .find(|r| r.id == cur)
        .ok_or((INVALID_PARAMS, "현재 문서를 찾을 수 없습니다".to_string()))
}

/// 문서 바이트를 로드해 HwpDocument로 만든다.
pub async fn load_doc(
    state: &AppState,
    row: &DocumentRow,
) -> Result<crate::wasm_api::HwpDocument, (i64, String)> {
    let bytes = state
        .store
        .get(&row.storage_key)
        .await
        .map_err(|e| (INTERNAL_ERROR, format!("문서 로드 실패: {e}")))?;
    crate::wasm_api::HwpDocument::from_bytes(&bytes)
        .map_err(|e| (INTERNAL_ERROR, format!("HWP 파싱 실패: {e}")))
}

/// 편집된 문서를 HWP로 직렬화해 저장하고 etag 갱신.
pub async fn save_doc(
    state: &AppState,
    row: &DocumentRow,
    doc: &mut crate::wasm_api::HwpDocument,
) -> Result<String, (i64, String)> {
    let bytes = doc
        .export_hwp_with_adapter()
        .map_err(|e| (INTERNAL_ERROR, format!("직렬화 실패: {e}")))?;
    let etag = state
        .store
        .put(&row.storage_key, &bytes)
        .await
        .map_err(|e| (INTERNAL_ERROR, format!("저장 실패: {e}")))?;
    state
        .db
        .update_document_etag(row.id, &etag)
        .await
        .map_err(|e| (INTERNAL_ERROR, format!("etag 갱신 실패: {e}")))?;
    Ok(etag)
}
