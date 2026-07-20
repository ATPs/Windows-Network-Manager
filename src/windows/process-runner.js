const { execFile } = require('node:child_process');
const { CommandExecutionError, CommandTimeoutError } = require('../utils/errors');

function runExecutable(file, args, options = {}) {
  return new Promise((resolve, reject) => {
    execFile(file, args, {
      windowsHide: true,
      shell: false,
      timeout: options.timeout ?? 20_000,
      maxBuffer: options.maxBuffer ?? 4 * 1024 * 1024,
      encoding: 'buffer',
      env: options.env ?? process.env,
    }, (error, stdout, stderr) => {
      if (error) {
        if (error.killed || error.code === 'ETIMEDOUT') {
          reject(new CommandTimeoutError(options.commandName || file));
          return;
        }
        reject(new CommandExecutionError(options.commandName || file, {
          exitCode: typeof error.code === 'number' ? error.code : null,
          stdout: decodeUtf8(stdout),
          stderr: decodeUtf8(stderr),
          cause: error,
        }));
        return;
      }
      resolve({ stdout, stderr, exitCode: 0 });
    });
  });
}

function decodeUtf8(buffer) {
  return Buffer.isBuffer(buffer) ? buffer.toString('utf8') : String(buffer || '');
}

module.exports = { decodeUtf8, runExecutable };
