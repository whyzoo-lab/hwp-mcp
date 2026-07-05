//! MCP 세션: Mcp-Session-Id별 현재 문서(doc_id)를 인메모리로 기억.
//! (단일 레플리카 전제. 다중 레플리카 시 sticky 세션 또는 공유 저장소 필요 — Phase 확장.)
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub struct Sessions {
    inner: Mutex<HashMap<String, Uuid>>,
}

impl Sessions {
    pub fn new() -> Self {
        Sessions {
            inner: Mutex::new(HashMap::new()),
        }
    }
    pub fn current_doc(&self, sid: &str) -> Option<Uuid> {
        self.inner.lock().unwrap().get(sid).copied()
    }
    pub fn set_current_doc(&self, sid: &str, doc: Uuid) {
        self.inner.lock().unwrap().insert(sid.to_string(), doc);
    }
}
