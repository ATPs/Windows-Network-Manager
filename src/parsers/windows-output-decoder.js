const iconv = require('iconv-lite');

function decodeWindowsOutput(buffer, codePages = [936, 65001, 437, 1252]) {
  if (!Buffer.isBuffer(buffer)) return String(buffer || '');
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(buffer).replace(/^\uFEFF/, '');
  } catch {
    // Continue through Windows code-page fallbacks.
  }

  for (const codePage of codePages) {
    const encoding = encodingForCodePage(codePage);
    if (!encoding || !iconv.encodingExists(encoding)) continue;
    const decoded = iconv.decode(buffer, encoding);
    if (!decoded.includes('\uFFFD')) return decoded.replace(/^\uFEFF/, '');
  }
  return buffer.toString('utf8').replace(/^\uFEFF/, '');
}

function encodingForCodePage(codePage) {
  const aliases = { 65001: 'utf8', 936: 'gbk', 950: 'big5', 932: 'shift_jis', 949: 'euc-kr' };
  return aliases[Number(codePage)] || `cp${Number(codePage)}`;
}

module.exports = { decodeWindowsOutput, encodingForCodePage };
