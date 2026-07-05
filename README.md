<p align="center">
  <img src="assets/logo/logo-256.png" alt="HWP MCP" width="112" />
</p>

<h1 align="center">HWP MCP</h1>

<p align="center">
  <strong>LLM을 위한 한글(HWP·HWPX) 문서 MCP 서버</strong><br/>
  <em>AI 에이전트가 HWP 문서를 사람 수준으로 <b>분석</b>하고, 서식을 지키며 <b>편집</b>하고, 한컴에서 열리는 표준 파일로 <b>저장</b>합니다.</em>
</p>

<p align="center">
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT" /></a>
  <a href="https://modelcontextprotocol.io/"><img src="https://img.shields.io/badge/MCP-2025--06--18-6E56CF.svg" alt="Model Context Protocol" /></a>
  <img src="https://img.shields.io/badge/Rust-pure%20(no%20C%2B%2B)-orange.svg" alt="Rust" />
  <img src="https://img.shields.io/badge/Hancom%202024-verified-1f6feb.svg" alt="Hancom 2024 verified" />
</p>

<p align="center">
  Developed by <a href="https://whyzoo.com"><strong>주식회사 와이주 (WhyZOO)</strong></a> · rhwp 오픈소스 엔진 기반
</p>

---

## 이게 뭔가요

**HWP MCP**는 [Model Context Protocol](https://modelcontextprotocol.io/) 서버입니다. Claude·GPT·Gemini 같은 LLM 에이전트에 **한글(HWP/HWPX) 문서를 다루는 도구 49종**을 노출해, 채팅/에이전트 워크플로우 안에서 한국 공문서·보고서·양식을 직접 읽고, 고치고, 만들 수 있게 합니다.

HWP는 한국 공공·행정 문서의 사실상 표준이지만 닫힌 바이너리 포맷이라 LLM 생태계에서 다루기 어려웠습니다. HWP MCP는 순수 Rust로 구현한 HWP 엔진(**rhwp**) 위에, 에이전트가 바로 쓸 수 있는 MCP 계층을 얹어 이 간극을 메웁니다. C++/Skia 의존성 없이 서버 한 대로 동작합니다.

> **핵심 설계 목표 3가지** — LLM이 HWP를 *PDF·이미지 수준으로 정확히* **① 분석**하고, 원본 서식을 그대로 지키며 **② 편집**하고, 한컴에서 100% 열리는 **③ 표준 저장**을 하도록.

## 왜 AI 개발자에게 매력적인가

| | HWP MCP |
|---|---|
| 🔎 **비전 분석** | `render_preview(format:"png")`가 페이지를 **MCP 이미지 블록**으로 반환 → LLM이 업로드한 PDF처럼 페이지를 **눈으로 직접 보고** 표·서식·레이아웃을 분석합니다. 텍스트 추출로는 놓치는 시각 정보까지 파악. |
| 🛡️ **"열리는 파일" 보장** | 저장 전 **바이트 구조 검증기**(레코드 경계·필수 스트림·CFB/ZIP 무결성)가 돌아, 응답에 `valid`/`validation`을 담습니다. 깨진 파일을 에이전트가 스스로 감지·재생성. |
| ✅ **한컴 2024 실기기 검증** | 그림 SHAPE_COMPONENT 레코드(196B), 이미지 사각형 인터리브 좌표, 셀 내부 이미지, 표 쪽분할·테두리 등 실제 한컴에서 열리는 경로를 실기기로 확인. |
| 🧩 **에이전트 친화 설계** | `search`로 위치를 얻고 → `format_text`/`set_paragraph_format`/표 편집으로 정확히 수정하는 **좌표 기반 편집 모델**. 스테이트리스 load→edit→save라 멀티유저·멀티세션 안전. |
| 📐 **변환 노하우 내장** | MCP `initialize` 응답의 `instructions`에 docx/PDF/HTML→HWP **고품질 변환 절차와 HTML 계약**을 담아, 어떤 에이전트·어떤 채팅방에서도 동일한 변환 품질을 재현. |
| 🔌 **표준 MCP** | claude.ai 웹 커넥터(OAuth 2.1), Claude Code·Codex·Gemini(Bearer 토큰), 로컬 stdio를 모두 지원. |

## 세 가지 축

### ① 분석 — HWP를 PDF/이미지처럼 정확하게 읽기
- `render_preview(format:"png"|"svg"|"pdf")` — 페이지를 이미지로 렌더. PNG는 LLM 비전 입력, SVG는 브라우저/아티팩트에서 한글 직접 렌더(서버 폰트 불필요).
- `read_document(format:"markdown")` — 표(`| a | b |`)·목록·제목까지 마크다운으로. `search`로 문자열·위치 탐색.
- `get_document_outline` · `list_tables` · `get_table_map` · `find_cell_by_label` · `list_fields` · `list_styles` — 구조·표·양식 필드·스타일을 기계 판독.
- `validate_document` · `doc_diff` — 구조 유효성 검사와 두 문서 차이 비교.

### ② 편집 — 원본 서식을 지키며 고치기
- `format_text`(굵게/기울임/밑줄/취소선/크기pt/색) · `set_paragraph_format`(정렬/줄간격/여백/들여쓰기).
- 표: `set_table_cell_text` · `merge_table_cells` · `split_table_cell` · 행/열 삽입·삭제 · `table_compute`(SUM/AVG 등).
- 텍스트: `insert_text` · `delete_text` · `replace_text` · `insert_html` · `add_heading`.
- 양식: `set_field_value`로 누름틀 채우기. 구조: 머리말/꼬리말·쪽번호·각주·책갈피.
- **서식 보존 하드닝**: 스테이트리스 편집에서 원본 서식이 유실되지 않도록 전 편집 명령을 감사·회귀로 고정(제목·머리셀 음영·테두리 등 무관 서식 전부 보존).

### ③ 저장 — 한컴 표준으로 완벽하게
- `export_document(format:"hwp"|"hwpx")` — presigned 다운로드 URL 발급, 응답에 구조 검증 결과 포함.
- HWP5(OLE/CFB)·HWPX(ZIP/XML) 무손실 라운드트립을 회귀 게이트로 검증.
- `repair_hwpx` · `official_lint`(공문서 서식 점검) 등 저장 품질 보조 도구.

## 도구 카탈로그 (HTTP 서버, 49종)

| 그룹 | 도구 |
|------|------|
| **문서 수명주기** | `create_document` · `list_documents` · `document_info` · `import_document` · `export_document` |
| **분석·조회** | `read_document` · `search` · `render_preview` · `validate_document` · `doc_diff` · `get_document_outline` · `list_tables` · `get_table_map` · `find_cell_by_label` · `list_fields` · `get_field_value` · `list_styles` · `export_html` |
| **텍스트 편집** | `insert_text` · `delete_text` · `replace_text` · `insert_html` · `add_heading` |
| **서식** | `format_text` · `set_paragraph_format` |
| **표** | `add_table` · `set_table_cell_text` · `merge_table_cells` · `split_table_cell` · `insert_table_row` · `insert_table_column` · `delete_table_row` · `delete_table_column` · `table_compute` |
| **페이지·구조** | `set_page_setup` · `set_header_footer` · `set_page_number` · `insert_footnote` · `add_bookmark` · `list_bookmarks` · `delete_bookmark` · `set_field_value` |
| **고수준 빌더** | `create_comparison_table` · `build_image_grid` · `build_organization_chart` · `build_meeting_nameplates` · `mail_merge` · `official_lint` · `repair_hwpx` |

> 로컬 stdio 서버(`rhwp-mcp`)는 이 중 핵심 부분집합(create/open/read/search/replace/insert/delete/insert_html/fields/export)을 제공합니다.

## 빠른 시작

두 가지 배포 형태가 있습니다.

- **`rhwp-mcp-http`** — 원격 멀티유저 HTTP 서버(OAuth 2.1 + Bearer, Postgres 메타 + S3 문서 저장). 팀/서비스용.
- **`rhwp-mcp`** — 로컬 단일사용자 stdio 서버. 개인 데스크톱/CI용.

### A. 호스팅된 서버에 에이전트 연결

**claude.ai 웹 커넥터** — 커넥터 설정에 서버 URL(`https://<YOUR_SERVER>/mcp`)만 등록 → 로그인 → 승인(OAuth 2.1). 별도 토큰 입력 없이 연결됩니다.

**Claude Code / Codex / Gemini** — `mcp-remote` stdio 브리지로 Bearer 토큰 연결:

```bash
npx -y mcp-remote https://<YOUR_SERVER>/mcp \
  --header "Authorization: Bearer <YOUR_TOKEN>"
```

### B. 직접 호스팅 (Docker)

```bash
cp .env.hwpmcp.example .env.hwpmcp        # 시크릿(DB/S3 비밀번호)을 실제 값으로 교체
#  공개 서버라면 RHWP_PUBLIC_BASE_URL=https://your-domain 설정, RHWP_AUTH_REQUIRED=true 유지

# 앱 + Postgres + MinIO 기동 (두 플래그 모두 필요)
docker compose --env-file .env.hwpmcp -f docker-compose.hwpmcp.yml up -d --build

# 스키마 마이그레이션 + 계정/토큰 발급
docker compose --env-file .env.hwpmcp -f docker-compose.hwpmcp.yml exec -T hwpmcp-app rhwp-mcp-http migrate
docker compose --env-file .env.hwpmcp -f docker-compose.hwpmcp.yml exec -T hwpmcp-app rhwp-mcp-http create-user alice 'strong-password'
docker compose --env-file .env.hwpmcp -f docker-compose.hwpmcp.yml exec -T hwpmcp-app rhwp-mcp-http issue-token alice
```

앱은 `127.0.0.1:8300`에 바인딩됩니다. 앞단에 리버스 프록시(Caddy/Nginx)로 TLS를 붙이고 `RHWP_PUBLIC_BASE_URL`을 공개 도메인으로 지정하세요(OAuth 메타데이터가 절대 URL을 사용합니다).

> **신뢰망 전용 간편 모드**: `RHWP_AUTH_REQUIRED=false`로 두면 인증 없이 단일 로컬 사용자로 동작합니다. **공개 인터넷에 노출된 서버에서는 절대 사용하지 마세요.**

### C. 로컬 stdio 서버

```bash
cargo build --release --features mcp --bin rhwp-mcp
```

MCP 클라이언트 설정에 등록(예):

```jsonc
{
  "mcpServers": {
    "hwp": { "command": "/absolute/path/to/rhwp-mcp" }
  }
}
```

## 변환 품질 — docx/PDF/HTML → HWP

서버는 `initialize` 응답에 **실전 검증된 변환 절차**를 담아 보냅니다(한컴 2024 실기기 통과 경로). 요지:

1. 원본에서 텍스트·글꼴/크기(pt)/색·정렬·문단 여백·표(열너비·병합·배경·테두리)·이미지·페이지 여백(mm)을 추출한다.
2. **인라인 스타일 HTML**로 조립한다 — 모든 텍스트를 `<span style="font-family:…;font-size:NNpt;color:#RRGGBB">`로 감싸고(생략 시 기본 글꼴·크기 적용), 표 셀에 `width(pt)`·`border`를 명시한다.
3. `create_document(name, html)` → `set_page_setup`으로 원본 여백 적용(빠뜨리면 여백이 크게 달라짐).
4. `render_preview(format:"png")`로 눈으로 확인 → `export_document`의 `valid`가 참인지 확인 후 다운로드.

이 계약을 따르면 어떤 에이전트든 표·병합셀·셀 배경·이미지·머리말/꼬리말까지 원본에 근접한 HWP를 만들 수 있습니다.

## 아키텍처

```
LLM 에이전트  ──MCP──▶  rhwp-mcp-http (axum)
(claude.ai /              │  ├─ OAuth 2.1 + Bearer 인증
 Claude Code /            │  ├─ 스테이트리스 load → edit → save
 Codex / Gemini)          │  ├─ Postgres  : 사용자·토큰 메타 (argon2)
                          │  ├─ S3/MinIO   : 문서 바이트 (앱 프록시 presigned)
                          │  └─ 구조 검증기 : 저장 전 "열리는 파일" 검사
                          ▼
                   rhwp 엔진 (순수 Rust)
                   파서 · 렌더러 · 직렬화기 · resvg 래스터
```

- **순수 Rust**: SVG→PNG 래스터는 `resvg`/`tiny-skia`로 처리해 C++/Skia 네이티브 의존성이 없습니다. 배포·크로스컴파일이 단순합니다.
- **스테이트리스 편집**: 매 호출마다 문서를 로드→편집→저장하므로 수평 확장과 멀티유저 격리가 자연스럽습니다.
- **엔진 재사용**: 웹 뷰어·CLI와 동일한 `DocumentCore` 편집/렌더 로직을 MCP가 그대로 사용합니다.

## rhwp 오픈소스 엔진 기반

HWP MCP의 문서 파싱·렌더링·직렬화는 오픈소스 HWP 엔진 **rhwp**(Rust + WebAssembly)를 기반으로 합니다. 저장소에는 rhwp 엔진, 웹 에디터(rhwp-studio), 브라우저/VS Code 확장 소스가 함께 포함되어 있어 MCP 서버부터 뷰어까지 전체 스택을 직접 빌드·검증할 수 있습니다.

로컬 개발(엔진 빌드·테스트·CLI)은 [`CLAUDE.md`](CLAUDE.md)와 [`CONTRIBUTING.md`](CONTRIBUTING.md)를 참고하세요.

## 개발

**HWP MCP는 [주식회사 와이주(WhyZOO)](https://whyzoo.com)가 개발합니다.** MCP 서버 계층은 오픈소스 rhwp 엔진 위에 구축되었으며, 전체가 MIT 라이선스로 공개됩니다. 기여·이슈·제안을 환영합니다.

## Notice

본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

## Trademark

"한글", "한컴", "HWP", "HWPX"는 주식회사 한글과컴퓨터의 등록 상표입니다. 본 프로젝트는 한글과컴퓨터와 제휴·후원·승인 관계가 없는 독립적인 오픈소스 프로젝트입니다.

"Hangul", "Hancom", "HWP", and "HWPX" are registered trademarks of Hancom Inc. This project is an independent open-source project with no affiliation, sponsorship, or endorsement by Hancom Inc.

## License

[MIT License](LICENSE) — rhwp 엔진 Copyright (c) 2025-2026 Edward Kim · MCP 서버 및 배포 Copyright (c) 2026 주식회사 와이주(WhyZOO)
