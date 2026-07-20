const crypto = require('node:crypto');
const { ValidationError } = require('../utils/errors');

const tokens = new Map();
const TOKEN_TTL_MS = 2 * 60 * 1000;

function issueConfirmation(kind, payload) {
  prune();
  const token = crypto.randomUUID();
  tokens.set(token, { kind, hash: hashPayload(payload), expiresAt: Date.now() + TOKEN_TTL_MS });
  return token;
}

function consumeConfirmation(token, kind, payload) {
  const record = tokens.get(token);
  tokens.delete(token);
  if (!record || record.expiresAt < Date.now() || record.kind !== kind || record.hash !== hashPayload(payload)) {
    throw new ValidationError('确认已失效，请重新预览操作。');
  }
}

function hashPayload(payload) {
  return crypto.createHash('sha256').update(stableStringify(payload)).digest('hex');
}

function stableStringify(value) {
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(',')}]`;
  if (value && typeof value === 'object') {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`).join(',')}}`;
  }
  return JSON.stringify(value);
}

function prune() {
  const now = Date.now();
  for (const [token, record] of tokens) if (record.expiresAt < now) tokens.delete(token);
}

module.exports = { consumeConfirmation, issueConfirmation, stableStringify };
