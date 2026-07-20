const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { createConfigStore, defaultConfig } = require('../src/config/config-store');

const directory = path.join(__dirname, 'tmp-config');

test.beforeEach(async () => {
  assert.ok(directory.startsWith(__dirname));
  await fs.rm(directory, { recursive: true, force: true });
});

test.after(async () => fs.rm(directory, { recursive: true, force: true }));

test('returns defaults when config does not exist', async () => {
  const result = await createConfigStore(directory).load();
  assert.deepEqual(result.config, defaultConfig());
  assert.equal(result.warning, null);
});

test('saves and reads configuration using replacement file', async () => {
  const store = createConfigStore(directory);
  const config = defaultConfig(); config.profiles.push({ id: 'one', name: '单位内网' });
  await store.save(config);
  assert.equal((await store.load()).config.profiles[0].name, '单位内网');
  assert.deepEqual((await fs.readdir(directory)).filter((name) => name.endsWith('.tmp')), []);
});

test('backs up corrupt configuration and recovers defaults', async () => {
  const store = createConfigStore(directory);
  await fs.mkdir(directory, { recursive: true });
  await fs.writeFile(store.filePath, '{broken', 'utf8');
  const result = await store.load();
  assert.deepEqual(result.config, defaultConfig());
  assert.match(result.warning, /已备份/);
  assert.ok((await fs.readdir(directory)).some((name) => name.includes('.corrupt-')));
});
