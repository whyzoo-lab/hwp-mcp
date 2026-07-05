//! 원격 서비스 런타임 설정(환경변수 기반).

/// S3(MinIO)와 Postgres 접속에 필요한 설정.
#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub s3_endpoint: String,
    pub s3_region: String,
    pub s3_bucket: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_use_path_style: bool,
    /// 프로덕션 HTTPS에서 세션 쿠키에 Secure 속성을 붙일지 여부.
    pub cookie_secure: bool,
    /// 공개 기준 URL(OAuth issuer/엔드포인트/메타데이터 절대 URL 생성용).
    pub public_base_url: String,
    /// `/mcp` 호출에 베어러 토큰/OAuth 인증을 요구할지 여부.
    ///
    /// 기본 true(멀티유저 호스팅). false 로 두면 인증 없이 단일 로컬 사용자로 동작하는
    /// **자가호스팅/신뢰망 모드**가 된다(계정·OAuth 불필요). 공개 인터넷에 노출된 서버에서는
    /// 절대 false 로 두면 안 된다 — 누구나 문서에 접근하게 된다.
    pub auth_required: bool,
}

impl Config {
    /// 프로세스 환경변수에서 설정을 읽는다. 누락 시 어떤 변수가 없는지 알려준다.
    pub fn from_env() -> Result<Config, String> {
        fn lookup(key: &str) -> Option<String> {
            std::env::var(key).ok()
        }
        Self::from_lookup(lookup)
    }

    /// 임의의 조회 함수(`key -> Option<value>`)로부터 설정을 읽는다. `from_env`가 사용하는
    /// 순수 헬퍼이며, 프로세스 env를 건드리지 않는 단위 테스트에서 직접 검증할 수 있다.
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        fn req(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Result<String, String> {
            lookup(key).ok_or_else(|| format!("환경변수 {key} 가 필요합니다"))
        }
        Ok(Config {
            database_url: req(&lookup, "RHWP_DATABASE_URL")?,
            s3_endpoint: req(&lookup, "RHWP_S3_ENDPOINT")?,
            s3_region: lookup("RHWP_S3_REGION").unwrap_or_else(|| "us-east-1".to_string()),
            s3_bucket: req(&lookup, "RHWP_S3_BUCKET")?,
            s3_access_key: req(&lookup, "RHWP_S3_ACCESS_KEY")?,
            s3_secret_key: req(&lookup, "RHWP_S3_SECRET_KEY")?,
            s3_use_path_style: lookup("RHWP_S3_PATH_STYLE")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(true),
            cookie_secure: lookup("RHWP_COOKIE_SECURE")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            public_base_url: lookup("RHWP_PUBLIC_BASE_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8300".to_string()),
            // 보안 기본값: 인증 요구. 명시적으로 false/0 일 때만 인증을 끈다.
            auth_required: lookup("RHWP_AUTH_REQUIRED")
                .map(|v| !(v == "false" || v == "0"))
                .unwrap_or(true),
        })
    }

    /// `HashMap` 등 명시적 맵으로부터 설정을 읽는다. 프로세스 env를 공유하지 않는
    /// 단위 테스트에서 사용한다.
    pub fn from_map(map: &std::collections::HashMap<String, String>) -> Result<Config, String> {
        Self::from_lookup(|key| map.get(key).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn full_map() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("RHWP_DATABASE_URL".to_string(), "postgres://u:p@localhost:5433/rhwp".to_string());
        m.insert("RHWP_S3_ENDPOINT".to_string(), "http://localhost:9002".to_string());
        m.insert("RHWP_S3_REGION".to_string(), "us-east-1".to_string());
        m.insert("RHWP_S3_BUCKET".to_string(), "rhwp-docs".to_string());
        m.insert("RHWP_S3_ACCESS_KEY".to_string(), "minioadmin".to_string());
        m.insert("RHWP_S3_SECRET_KEY".to_string(), "minioadmin".to_string());
        m.insert("RHWP_S3_PATH_STYLE".to_string(), "true".to_string());
        m
    }

    #[test]
    fn from_map_reads_all_fields() {
        let c = Config::from_map(&full_map()).expect("config");
        assert_eq!(c.database_url, "postgres://u:p@localhost:5433/rhwp");
        assert_eq!(c.s3_bucket, "rhwp-docs");
        assert!(c.s3_use_path_style);
    }

    #[test]
    fn auth_required_defaults_true_and_toggles_off() {
        // 기본값: 인증 요구(보안 기본값).
        assert!(Config::from_map(&full_map()).unwrap().auth_required);
        // 명시적 false/0 일 때만 꺼진다.
        for off in ["false", "0"] {
            let mut m = full_map();
            m.insert("RHWP_AUTH_REQUIRED".to_string(), off.to_string());
            assert!(!Config::from_map(&m).unwrap().auth_required, "{off} 는 인증 off");
        }
        // 그 외 값(빈/true/오타)은 안전하게 인증 유지.
        let mut m = full_map();
        m.insert("RHWP_AUTH_REQUIRED".to_string(), "yes".to_string());
        assert!(Config::from_map(&m).unwrap().auth_required);
    }

    #[test]
    fn from_map_missing_database_url_reports_which() {
        let mut m = full_map();
        m.remove("RHWP_DATABASE_URL");
        let err = Config::from_map(&m).err().expect("DB URL 누락은 에러여야 함");
        assert!(err.contains("RHWP_DATABASE_URL"), "에러 메시지에 변수명 포함: {err}");
    }
}
