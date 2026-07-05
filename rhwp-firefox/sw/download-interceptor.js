// 다운로드 가로채기 (Firefox 버전)
// - onCreated: url/mime/referrer 기반 1차 판정 (filename 미확정일 수 있음)
// - onChanged: filename 확정 시 2차 판정
// - browser.downloads.search 로 최신 DownloadItem 재조회
// - handled 집합으로 동일 다운로드 중복 처리 방지
//
// #207: 판정 로직은 rhwp-shared/sw/download-interceptor-common.js 와 공유.
// Chrome 과 동일한 DEXT5 블랙리스트 / MIME 힌트 / finalUrl 검사 자동 적용.

import { openViewer } from './viewer-launcher.js';
import { shouldInterceptDownload } from './download-interceptor-common.js';

const handled = new Set();     // 이미 처리된 downloadId
const workerStartedAt = Date.now();
const DOWNLOAD_FRESH_GRACE_MS = 5_000;
// #1498: onCreated 로 관측한 다운로드 id (= service worker 기동 이후 새로 시작된 다운로드).
// onChanged 는 과거 다운로드 기록에도 발화할 수 있으므로, seen 에 있는 id 에 한해서만 재판정한다.
// SW 재기동 후 과거 항목이 뷰어로 다발 열리던 회귀를 막는다 (#1471/#1480 회귀 정정).
const seen = new Set();

function parseDownloadTime(value) {
  if (typeof value !== 'string' || value.length === 0) return null;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms : null;
}

/**
 * #1498 v2: onCreated 로 들어온 항목도 과거 다운로드 기록일 수 있다.
 * startTime/endTime 이 event page 기동 시각보다 충분히 오래 전이면 재시작/복원된 과거 항목으로 보고 무시한다.
 */
function isFreshDownloadItem(item) {
  if (!item) return false;
  const startMs = parseDownloadTime(item.startTime);
  if (startMs === null) return true;
  const freshBoundary = workerStartedAt - DOWNLOAD_FRESH_GRACE_MS;
  if (startMs < freshBoundary) return false;

  const endMs = parseDownloadTime(item.endTime);
  if (endMs !== null && endMs < freshBoundary) return false;

  return true;
}

export function setupDownloadInterceptor() {
  // 1차: 다운로드 시작 시 url/mime/referrer 로 즉시 판정
  browser.downloads.onCreated.addListener((item) => {
    if (!isFreshDownloadItem(item)) return;
    seen.add(item.id);
    if (handled.has(item.id)) return;

    if (shouldInterceptDownload(item)) {
      handled.add(item.id);
      handleHwpDownload(item);
    }
    // 판정 불가 시: filename 확정될 때 onChanged에서 재판정
  });

  // 2차: filename 확정 시 재판정
  browser.downloads.onChanged.addListener(async (delta) => {
    // #1498: onCreated 로 관측한(= 새로 시작된) 다운로드만 재판정한다. onChanged 단독으로
    // 들어온 과거 기록 항목은 seen 에 없으므로 뷰어를 열지 않는다.
    if (seen.has(delta.id) && !handled.has(delta.id) && delta.filename?.current) {
      try {
        const [item] = await browser.downloads.search({ id: delta.id });
        if (isFreshDownloadItem(item) && shouldInterceptDownload(item)) {
          handled.add(delta.id);
          handleHwpDownload(item);
        }
      } catch (err) {
        console.error('[rhwp] 다운로드 항목 재조회 오류:', err);
      }
    }

    // 완료/에러 시 handled/seen 정리 (메모리 누수 방지)
    if (delta.state?.current === 'complete' || delta.error) {
      setTimeout(() => {
        handled.delete(delta.id);
        seen.delete(delta.id);
      }, 30000);
    }
  });
}

async function handleHwpDownload(item) {
  try {
    const settings = await browser.storage.sync.get({ autoOpen: true });
    if (!settings.autoOpen) return;

    if (item.fileSize > 50 * 1024 * 1024) {
      console.warn(
        `[rhwp] 대용량 파일: ${item.filename} (${(item.fileSize / 1024 / 1024).toFixed(1)}MB)`
      );
    }

    openViewer({
      url: item.url,
      filename: item.filename,
    });
  } catch (err) {
    console.error('[rhwp] 다운로드 인터셉터 오류:', err);
  }
}
