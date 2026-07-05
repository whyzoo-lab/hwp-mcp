//! 앱 프록시 다운로드.
//!
//! presigned URL은 서명에 내부 endpoint 호스트(`hwpmcp-minio:9000`)가 들어가
//! 외부(브라우저/에이전트)에서 열 수 없다. 대신 짧은 수명의 불투명 토큰을 발급하고,
//! 앱이 그 토큰으로 S3(내부 minio)에서 바이트를 읽어 스트리밍한다. minio는 계속
//! 비공개로 두면서 `{public_base_url}/download/{token}` 링크로 어디서나 내려받는다.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;

use super::auth;
use super::server::AppState;

struct Entry {
    storage_key: String,
    filename: String,
    expires_epoch: i64,
}

/// 다운로드 토큰 → (storage_key, filename) 인메모리 매핑(단일 레플리카 전제, 짧은 TTL).
#[derive(Default)]
pub struct DownloadTokens {
    inner: Mutex<HashMap<String, Entry>>,
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl DownloadTokens {
    pub fn new() -> Self {
        DownloadTokens {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// 다운로드 토큰 발급. filename은 브라우저 저장 이름(확장자 포함).
    pub fn issue(&self, storage_key: &str, filename: &str, ttl_secs: i64) -> String {
        let token = auth::generate_token();
        let now = now_epoch();
        let mut m = self.inner.lock().unwrap();
        m.retain(|_, e| e.expires_epoch > now); // 만료분 정리
        m.insert(
            token.clone(),
            Entry {
                storage_key: storage_key.to_string(),
                filename: filename.to_string(),
                expires_epoch: now + ttl_secs,
            },
        );
        token
    }

    fn resolve(&self, token: &str) -> Option<(String, String)> {
        let m = self.inner.lock().unwrap();
        let e = m.get(token)?;
        if e.expires_epoch <= now_epoch() {
            return None;
        }
        Some((e.storage_key.clone(), e.filename.clone()))
    }
}

/// 업로드 토큰 → storage_key 인메모리 매핑(단일 레플리카 전제, 짧은 TTL).
/// presigned PUT이 내부 호스트라 외부에서 못 올리는 문제를 앱 프록시로 해결한다.
#[derive(Default)]
pub struct UploadTokens {
    inner: Mutex<HashMap<String, (String, i64)>>,
}

impl UploadTokens {
    pub fn new() -> Self {
        UploadTokens {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// 업로드 토큰 발급(→ storage_key).
    pub fn issue(&self, storage_key: &str, ttl_secs: i64) -> String {
        let token = auth::generate_token();
        let now = now_epoch();
        let mut m = self.inner.lock().unwrap();
        m.retain(|_, (_, exp)| *exp > now);
        m.insert(token.clone(), (storage_key.to_string(), now + ttl_secs));
        token
    }

    fn resolve(&self, token: &str) -> Option<String> {
        let m = self.inner.lock().unwrap();
        let (key, exp) = m.get(token)?;
        if *exp <= now_epoch() {
            return None;
        }
        Some(key.clone())
    }
}

/// PUT /upload/{token} — 토큰이 가리키는 storage_key로 요청 바디를 저장한다.
pub async fn upload(
    State(state): State<AppState>,
    Path(token): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let key = match state.uploads.resolve(&token) {
        Some(k) => k,
        None => {
            return (
                StatusCode::NOT_FOUND,
                "업로드 링크가 만료되었거나 유효하지 않습니다",
            )
                .into_response()
        }
    };
    match state.store.put(&key, &body).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// RFC 5987 `filename*=UTF-8''` 용 퍼센트 인코딩.
fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// GET /download/{token} — 토큰을 확인하고 S3 바이트를 스트리밍한다.
pub async fn download(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    let (key, filename) = match state.downloads.resolve(&token) {
        Some(v) => v,
        None => {
            return (
                StatusCode::NOT_FOUND,
                "링크가 만료되었거나 유효하지 않습니다",
            )
                .into_response()
        }
    };
    let bytes = match state.store.get(&key).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "파일을 찾을 수 없습니다").into_response(),
    };
    // 확장자 기반 ASCII 폴백 + UTF-8 filename*.
    let ext = filename.rsplit('.').next().unwrap_or("hwp");
    let cd = format!(
        "attachment; filename=\"file.{ext}\"; filename*=UTF-8''{}",
        pct_encode(&filename)
    );
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_DISPOSITION, cd),
        ],
        bytes,
    )
        .into_response()
}
