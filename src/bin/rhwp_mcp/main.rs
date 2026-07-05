//! rhwp MCP 서버 실행 파일. stdio JSON-RPC 루프를 시작한다.

fn main() {
    rhwp::mcp::serve_stdio();
}
