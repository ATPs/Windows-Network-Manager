const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
const files = ['main.js', 'preload.js', 'renderer.js', ...walk(path.join(root, 'src')).map((file) => path.relative(root, file))];

for (const relative of files) {
  const isBrowserModule = relative === 'renderer.js' || relative.startsWith(`src${path.sep}renderer${path.sep}`);
  const absolute = path.join(root, relative);
  const result = isBrowserModule
    ? spawnSync(process.execPath, ['--input-type=module', '--check'], { input: fs.readFileSync(absolute), encoding: 'utf8' })
    : spawnSync(process.execPath, ['--check', absolute], { encoding: 'utf8' });
  if (result.status !== 0) {
    process.stderr.write(`Syntax check failed: ${relative}\n${result.stderr}`);
    process.exit(result.status || 1);
  }
}

console.log(`Syntax check passed for ${files.length} JavaScript files.`);

function walk(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const fullPath = path.join(directory, entry.name);
    return entry.isDirectory() ? walk(fullPath) : entry.isFile() && entry.name.endsWith('.js') ? [fullPath] : [];
  });
}
