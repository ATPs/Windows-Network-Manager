function toArray(value) {
  if (value === null || value === undefined) return [];
  return Array.isArray(value) ? value : [value];
}

function toBoolean(value) {
  if (typeof value === 'boolean') return value;
  const normalized = String(value ?? '').toLowerCase();
  return ['true', 'enabled', '1', 'yes'].includes(normalized);
}

function optionalString(value) {
  const text = value === null || value === undefined ? '' : String(value).trim();
  return text || null;
}

function uniqueStrings(values) {
  return [...new Set(toArray(values).map(optionalString).filter(Boolean))];
}

module.exports = { optionalString, toArray, toBoolean, uniqueStrings };
