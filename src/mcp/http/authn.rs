//! 베어러 토큰 인증: Authorization 헤더 → user_id.
use axum::http::HeaderMap;
use uuid::Uuid;

use super::db::Db;

/// Authorization: Bearer <token> 를 파싱해 검증하고 user_id를 반환. 실패 시 Err(()).
pub async fn authenticate(db: &Db, headers: &HeaderMap) -> Result<Uuid, ()> {
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(())?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .ok_or(())?;
    match db.authenticate_token(token).await {
        Ok(Some(uid)) => Ok(uid),
        _ => Err(()),
    }
}
