#!/bin/bash
# ─────────────────────────────────────────────────────
# prepare-github.sh
# GitLab 리포 → GitHub 공개 리포로 선별 복사
#
# 사용법:
#   ./scripts/prepare-github.sh [대상 경로]
#   기본: /home/edward/mygithub/rhwp
# ─────────────────────────────────────────────────────
set -euo pipefail

SRC="$(cd "$(dirname "$0")/.." && pwd)"
DST="${1:-/home/edward/mygithub/rhwp}"
EXCLUDE_FILE="$SRC/scripts/github-exclude.txt"

echo "=== rhwp GitHub 공개 리포 준비 ==="
echo "  소스: $SRC"
echo "  대상: $DST"
echo ""

# ── 1. 대상 디렉토리 생성 ──
mkdir -p "$DST"

# ── 2. rsync 선별 복사 ──
echo "[1/5] 파일 복사 중..."
rsync -av --delete \
    --exclude-from="$EXCLUDE_FILE" \
    "$SRC/" "$DST/" \
    --quiet

echo "  복사 완료"

# ── 3. 공개용 파일로 교체 ──
echo "[2/5] 공개용 파일 교체..."
if [ -f "$SRC/CLAUDE.public.md" ]; then
    cp "$SRC/CLAUDE.public.md" "$DST/CLAUDE.md"
    echo "  CLAUDE.public.md → CLAUDE.md"
else
    echo "  ⚠ CLAUDE.public.md 없음 — CLAUDE.md 제외 상태 유지"
fi
if [ -f "$SRC/README.public.md" ]; then
    cp "$SRC/README.public.md" "$DST/README.md"
    echo "  README.public.md → README.md"
fi

# ── 4. .env.docker.example 복사 ──
echo "[3/5] .env.docker.example 복사..."
if [ -f "$SRC/.env.docker.example" ]; then
    cp "$SRC/.env.docker.example" "$DST/.env.docker.example"
    echo "  .env.docker.example 복사 완료"
else
    echo "  ⚠ .env.docker.example 없음 — 건너뜀"
fi

# ── 5. 민감 파일 잔여 확인 ──
echo "[4/5] 민감 파일 잔여 확인..."
SENSITIVE_FOUND=0

check_not_exists() {
    if [ -e "$DST/$1" ]; then
        echo "  ✗ 민감 파일 발견: $1"
        SENSITIVE_FOUND=1
    fi
}

if grep -q "q1w2e3r4\|192.168.2.154\|gpu_key" "$DST/CLAUDE.md" 2>/dev/null; then
    echo "  ✗ CLAUDE.md에 민감 정보 포함!"
    SENSITIVE_FOUND=1
else
    echo "  ✓ CLAUDE.md 민감 정보 없음"
fi
check_not_exists ".env.docker"
check_not_exists "hwp_webctl/"
check_not_exists "mydocs/manual/hwp/"
check_not_exists "mydocs/manual/hwpctl/"
check_not_exists "mydocs/convers/"
check_not_exists "samples/kps-ai.hwp"
check_not_exists "samples/bodo-01.hwp"
check_not_exists "samples/gonggo-01.hwp"
check_not_exists "samples/exam_math.hwp"

if [ "$SENSITIVE_FOUND" -eq 0 ]; then
    echo "  ✓ 민감 파일 없음"
else
    echo "  ⚠ 민감 파일이 발견되었습니다. github-exclude.txt를 확인하세요."
fi

# ── 6. 결과 요약 ──
echo "[5/5] 결과 요약..."
FILE_COUNT=$(find "$DST" -type f | wc -l)
DIR_COUNT=$(find "$DST" -type d | wc -l)
echo ""
echo "=== 완료 ==="
echo "  대상: $DST"
echo "  파일: ${FILE_COUNT}개"
echo "  디렉토리: ${DIR_COUNT}개"
echo ""
echo "다음 단계:"
echo "  cd $DST"
echo "  git init && git add -A && git commit -m 'Initial commit: rhwp v0.5.0'"
echo "  git remote add origin git@github.com:{user}/rhwp.git"
echo "  git push -u origin main"
