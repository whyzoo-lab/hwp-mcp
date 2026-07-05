// rhwp 파일 시그니처(매직 넘버) 검증 모듈 — Chrome/Safari 공통
'use strict';

/** HWP 5.0 (OLE2 Compound Document) 매직 넘버 */
const HWP_SIGNATURE = [0xD0, 0xCF, 0x11, 0xE0];

/** HWPX (ZIP) 매직 넘버 */
const HWPX_SIGNATURE = [0x50, 0x4B, 0x03, 0x04];

/**
 * 바이트 배열의 시작이 주어진 시그니처와 일치하는지 확인한다.
 * @param {Uint8Array} bytes
 * @param {number[]} signature
 * @returns {boolean}
 */
function matchesSignature(bytes, signature) {
  if (bytes.length < signature.length) return false;
  for (let i = 0; i < signature.length; i++) {
    if (bytes[i] !== signature[i]) return false;
  }
  return true;
}

/**
 * 파일 데이터가 HWP 또는 HWPX 파일인지 매직 넘버로 확인한다.
 * @param {ArrayBuffer|Uint8Array} data
 * @returns {{ isHwp: boolean, format: 'hwp'|'hwpx'|null }}
 */
function verifyHwpSignature(data) {
  const bytes = data instanceof Uint8Array ? data : new Uint8Array(data);
  if (bytes.length < 4) {
    return { isHwp: false, format: null };
  }

  if (matchesSignature(bytes, HWP_SIGNATURE)) {
    return { isHwp: true, format: 'hwp' };
  }
  if (matchesSignature(bytes, HWPX_SIGNATURE)) {
    return { isHwp: true, format: 'hwpx' };
  }

  return { isHwp: false, format: null };
}

// 내보내기
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { HWP_SIGNATURE, HWPX_SIGNATURE, verifyHwpSignature };
}
