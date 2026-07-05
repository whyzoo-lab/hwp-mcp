# samples/ — 테스트용 HWP/HWPX 파일 provenance & 라이선스

이 디렉터리의 파일들은 rhwp 파서·렌더러·직렬화기의 **회귀 테스트 픽스처**입니다.

> ⚠️ **중요**: rhwp 소스 코드는 MIT 라이선스지만, **여기의 샘플 문서 파일은 각자의 저작권을
> 그대로 유지**합니다. MIT는 rhwp가 작성한 코드에만 적용되며, 제3자·기관이 저작권을 가진
> 문서까지 MIT로 재라이선스하지 않습니다. 공개 배포 전 아래 분류에 따라 **저작권 확인이 필요한
> 파일을 제거하거나 재배포 허가를 확보**해야 합니다.

## 분류

### A. 저자 제작 테스트 픽스처 — 재배포 안전
특정 기능을 검증하려 저자가 직접 만든 합성 문서. 저작물성 있는 제3자 콘텐츠 없음.

- 회귀 자동생성: `re-*` (한글/라틴/숫자/정렬/폰트/크기 조합 — 사소한 테스트 글자만)
- 레이아웃/조판: `lseg-*`, `para-*`, `text-align*`, `landscape-001`, `종이기준`, `쪽기준`
- 표: `table-001/004/complex`, `multi-table-*`, `inner-table-*`, `hwp_table_test*`, `table-ipc`, `table-vpos-01`, `셀보호*`, `calc-cell`
- 그림/도형: `pic-*`, `test-image*`, `bitmap`, `draw-group`, `group-*`, `shape-*`, `mix-shape-01`, `img-start-001`, `water-mark`, `tb-img-03`, `textbox-under-image`, `투명도0-50*`, `hcar-001`, `h-pen-01`
- 글자처럼취급(TAC): `tac-case-*`, `tac-img-*`, `tac-host-spacing`, `ta-pic-001-r*`, `samples/tac-verify/*`
- 수식: `eq-01/002`, `equation-lim`, `atop-equation-01`, `math-001`, `수식-문자처럼취급-아님`
- 각주/미주/필드/기타: `footnote-*`, `endnote-01`, `field-01*`, `누름틀-2024`, `form-01/02`, `shift-return`, `pua-test`, `task-001`, `mel-001`, `p122`, `pr-149`, `loading-fail-01`
- 차트: `samples/chart/*` (차트 유형별 샘플 — 저자 생성)

### B. 한컴(Hancom) 제공 샘플/템플릿 — 재배포 권리 확인 필요
한컴 오피스 기본 템플릿·SDK·포맷 스펙에서 유래한 것으로 보이는 파일. 한컴 저작권일 수 있어
**재배포 허가 확인 또는 제거** 권장.

- 포맷 스펙/샘플: `hwpspec.hwp`, `hwp3-sample*.hwp/.hwpx`, `hwpx_sample2.*`, `한셀OLE.hwp`
- hwpctl API 문서: `hwpctl_API_v2.4.hwp`, `hwpctl_Action_Table__v1.1.hwp`, `hwpctl_ParameterSetID_Item_v1.2.hwp`
- 기본 템플릿: `samples/basic/*` (BlogForm_*, calendar_*, Worldcup_FIFA2010, NewYear_s_Day, request, interview, treatise sample 등 — 한컴 내장 템플릿 추정)

### C. 제3자·기관 문서 — 저작권 위험 (제거 또는 허가 필요)
외부 저작권이 있을 가능성이 높은 실제 문서. **공개 재배포 전 제거하거나 명시적 허가 확보 필수.**

| 파일 | 성격 | 비고 |
|------|------|------|
| `2022년 국립국어원 업무계획.hwp` | 정부기관 문서 | 공공누리(KOGL) 여부 확인 |
| `k-water-rfp.hwp`, `k-water-rfp-2024.hwp` | 공기업 RFP | 저작권/기밀 확인 |
| `3-09월_교육_통합_*`, `3-10월_교육_통합_*`, `3-11월_실전_통합_*` (hwp+hwpx 다수) | 모의고사/교육자료 | **사설 저작물 가능성 높음** |
| `21_언어_기출_편집가능본.hwp` | 기출문제 | 저작권 확인 |
| `exam_kor/eng/math/science/social*.hwp`, `exam-kor-*p.hwp`, `exam_math_*.hwp` | 시험지 | 저작권 확인 |
| ~~`[2027] 온새미로 1 본교재.hwp/.hwpx`~~ | 교재(출판물, "박윤수 지음" 상업 수능 교재) | **✅ 제거됨(2026-07)** — 저작권 침해. 전용 테스트(issue_1196/1271/1440, issue_1392 일부, hwpx baseline 항목)도 함께 제거. 대체는 라이선스 명확한 문서로 재픽스처 필요 |
| `통합재정통계(2010.11월/2011.10월/2014.8월).hwp` | 정부 재정통계 | 공공누리 여부 확인 |
| `2025년 기부·답례품 실적 지자체 보고서_양식.hwpx` | 지자체 양식 | 공공누리 여부 확인 |
| `복학원서.hwp` | 학교 서식 | 실제 기관 서식 여부 확인 |
| `20250130-hongbo*.hwp`, `honbo-save.hwp` | 홍보물 추정 | 출처 확인 |
| `issue-986-receipt.hwp` | 영수증 | **개인정보 포함 여부 확인** |
| `el-school-001.hwp`, `biz_plan.hwp`, `aift.hwp`, `kps-ai.hwp`, `synam-001.hwp`, `KTX.hwp`, `2026_oss_rst.hwp` | 출처 불명 실문서 추정 | 개별 확인 |

## 권장 조치 (공개 전)

1. **C 항목**은 제거하거나(권장) 재배포 허가를 확보한다. 정부 문서는 [공공누리(KOGL)](https://www.kogl.or.kr) 유형을 확인 — 제1유형이면 출처표시 후 사용 가능.
2. **B 항목**은 한컴 재배포 정책을 확인하거나, 동일 기능을 검증하는 **저자 생성 대체 픽스처**로 교체한다.
3. 제거 시 해당 파일을 참조하는 테스트를 함께 갱신한다(예: `tests/hwpx_roundtrip_baseline`가 `samples/hwpx/` 전수를 스캔; `p122`/`KTX` 등 특정 파일을 로드하는 테스트 존재).
4. **개인정보(PII)** 포함 문서(영수증·원서·홍보물 등)는 반드시 확인·마스킹 또는 제거한다.

## 새 샘플 추가 규칙

- 가능한 한 **저자가 직접 만든 최소 픽스처**를 쓴다(실제 저작물 대신).
- 외부 문서가 꼭 필요하면 **재배포 허가가 명확한 것**(공공누리 제1유형, CC0/CC-BY, 명시적 허가)만 추가하고, 이 표에 출처·라이선스를 기록한다.
- PII를 포함하지 않는다.
