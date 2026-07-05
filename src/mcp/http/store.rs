//! MinIO(S3 호환) 문서 스토리지: put/get + presigned URL.

use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};

use super::config::Config;

/// S3 버킷 핸들 래퍼.
pub struct Store {
    bucket: Box<Bucket>,
}

impl Store {
    /// 버킷에 연결한다. 없으면 생성 시도(멱등).
    pub async fn connect(cfg: &Config) -> Result<Store, String> {
        let region = Region::Custom {
            region: cfg.s3_region.clone(),
            endpoint: cfg.s3_endpoint.clone(),
        };
        let creds = Credentials::new(
            Some(&cfg.s3_access_key),
            Some(&cfg.s3_secret_key),
            None,
            None,
            None,
        )
        .map_err(|e| format!("자격증명 오류: {e}"))?;

        let mut bucket = Bucket::new(&cfg.s3_bucket, region.clone(), creds.clone())
            .map_err(|e| format!("버킷 핸들 오류: {e}"))?;
        if cfg.s3_use_path_style {
            bucket.set_path_style();
        }

        // 존재 확인 겸 생성 시도. rust-s3 "fail-on-err" 피처로 인해 버킷이 이미 있으면
        // (409 BucketAlreadyOwnedByYou 등) Err가 반환되지만, 아래 `let _ =`로 명시적으로
        // 무시한다 — 존재 확인 목적일 뿐 실패로 취급하지 않아 connect는 멱등을 유지한다.
        let _ = Bucket::create_with_path_style(
            &cfg.s3_bucket,
            region,
            creds,
            BucketConfiguration::default(),
        )
        .await;

        Ok(Store { bucket })
    }

    pub async fn put(&self, key: &str, bytes: &[u8]) -> Result<String, String> {
        let resp = self
            .bucket
            .put_object(key, bytes)
            .await
            .map_err(|e| format!("put 실패: {e}"))?;
        // etag는 헤더에서; rust-s3 응답에 없으면 콘텐츠 길이로 대체
        let etag = resp
            .headers()
            .get("etag")
            .cloned()
            .unwrap_or_else(|| format!("{}", bytes.len()));
        Ok(etag.trim_matches('"').to_string())
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>, String> {
        let resp = self
            .bucket
            .get_object(key)
            .await
            .map_err(|e| format!("get 실패: {e}"))?;
        Ok(resp.bytes().to_vec())
    }

    pub async fn presigned_put(&self, key: &str, expiry_secs: u32) -> Result<String, String> {
        self.bucket
            .presign_put(key, expiry_secs, None, None)
            .await
            .map_err(|e| format!("presign put 실패: {e}"))
    }

    pub async fn presigned_get(&self, key: &str, expiry_secs: u32) -> Result<String, String> {
        self.bucket
            .presign_get(key, expiry_secs, None)
            .await
            .map_err(|e| format!("presign get 실패: {e}"))
    }
}
