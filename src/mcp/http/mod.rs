//! rhwp 원격 HTTP MCP 서비스. feature = "mcp-http"로 활성화.
//! Phase 1: 설정/DB/스토리지/인증 기반. HTTP 서빙은 Phase 2.

pub mod auth;
pub mod authn;
pub mod config;
pub mod db;
pub mod doc_ctx;
pub mod download;
pub mod oauth;
pub mod rpc;
pub mod rtools;
pub mod server;
pub mod session;
pub mod store;
pub mod validate;
pub mod web;
pub use server::serve;
