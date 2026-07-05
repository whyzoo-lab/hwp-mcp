//! rhwp 원격 서비스 관리 CLI.

use rhwp::mcp::http::{config::Config, db::Db};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("check-config") => {
            if let Err(e) = Config::from_env() {
                eprintln!("설정 오류: {e}");
                std::process::exit(1);
            }
            println!("설정 OK");
        }
        Some("migrate") => {
            let cfg = load_or_die();
            let db = connect_or_die(&cfg).await;
            db.migrate().await.unwrap_or_else(|e| die(&e));
            println!("마이그레이션 완료");
        }
        Some("create-user") => {
            let name = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| die("사용법: create-user <name> <password> [--admin]"));
            let pw = args
                .get(3)
                .cloned()
                .unwrap_or_else(|| die("비밀번호가 필요합니다"));
            // 4번째 이후 인자에 --admin 이 있으면 관리자로 생성.
            let is_admin = args.iter().skip(4).any(|a| a == "--admin");
            let cfg = load_or_die();
            let db = connect_or_die(&cfg).await;
            db.migrate().await.unwrap_or_else(|e| die(&e));
            let hash = rhwp::mcp::http::auth::hash_secret(&pw).unwrap_or_else(|e| die(&e));
            let id = db
                .create_user(&name, &hash, is_admin)
                .await
                .unwrap_or_else(|e| die(&e));
            let role = if is_admin { " [관리자]" } else { "" };
            println!("사용자 생성: {name} ({id}){role}");
        }
        Some("set-password") => {
            let name = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| die("사용법: set-password <name> <password>"));
            let pw = args
                .get(3)
                .cloned()
                .unwrap_or_else(|| die("비밀번호가 필요합니다"));
            let cfg = load_or_die();
            let db = connect_or_die(&cfg).await;
            db.migrate().await.unwrap_or_else(|e| die(&e));
            let hash = rhwp::mcp::http::auth::hash_secret(&pw).unwrap_or_else(|e| die(&e));
            let n = db.set_password(&name, &hash).await.unwrap_or_else(|e| die(&e));
            if n == 0 {
                die(&format!("해당 사용자가 없습니다: {name}"));
            }
            println!("비밀번호 변경 완료: {name}");
        }
        Some("issue-token") => {
            let name = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| die("사용법: issue-token <name>"));
            let cfg = load_or_die();
            let db = connect_or_die(&cfg).await;
            let (uid, _) = db
                .find_user_by_name(&name)
                .await
                .unwrap_or_else(|e| die(&e))
                .unwrap_or_else(|| die("해당 사용자가 없습니다"));
            let token = rhwp::mcp::http::auth::generate_token();
            let hash = rhwp::mcp::http::auth::hash_secret(&token).unwrap_or_else(|e| die(&e));
            db.issue_token(uid, &hash).await.unwrap_or_else(|e| die(&e));
            println!("발급된 토큰(한 번만 표시): {token}");
        }
        Some("list-users") => {
            let cfg = load_or_die();
            let db = connect_or_die(&cfg).await;
            let c = db.pool.get().await.unwrap_or_else(|e| die(&e.to_string()));
            let rows = c
                .query(
                    "SELECT name, created_at FROM users ORDER BY created_at",
                    &[],
                )
                .await
                .unwrap_or_else(|e| die(&e.to_string()));
            for r in rows {
                let n: String = r.get(0);
                println!("{n}");
            }
        }
        Some("serve") => {
            let cfg = load_or_die();
            if let Err(e) = rhwp::mcp::http::serve(cfg).await {
                die(&e);
            }
        }
        _ => {
            eprintln!("사용법: rhwp-mcp-http <check-config|migrate|create-user|set-password|issue-token|list-users|serve>");
            std::process::exit(2);
        }
    }
}

fn load_or_die() -> Config {
    Config::from_env().unwrap_or_else(|e| die(&e))
}
async fn connect_or_die(cfg: &Config) -> Db {
    Db::connect(cfg).await.unwrap_or_else(|e| die(&e))
}
fn die(msg: &str) -> ! {
    eprintln!("오류: {msg}");
    std::process::exit(1);
}
