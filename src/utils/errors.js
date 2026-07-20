class AppError extends Error {
  constructor(code, message, options = {}) {
    super(message, options.cause ? { cause: options.cause } : undefined);
    this.name = this.constructor.name;
    this.code = code;
    this.details = options.details;
    this.recoverable = options.recoverable ?? true;
  }
}

class ValidationError extends AppError {
  constructor(message, details) {
    super('VALIDATION_ERROR', message, { details });
  }
}

class PermissionError extends AppError {
  constructor(message = '此操作需要以管理员身份运行。') {
    super('PERMISSION_REQUIRED', message);
  }
}

class CommandTimeoutError extends AppError {
  constructor(commandName) {
    super('COMMAND_TIMEOUT', `${commandName} 操作超时。`, { details: { commandName } });
  }
}

class CommandExecutionError extends AppError {
  constructor(commandName, options = {}) {
    super('COMMAND_FAILED', `${commandName} 执行失败。`, {
      details: {
        commandName,
        exitCode: options.exitCode ?? null,
        stderr: sanitizeCommandText(options.stderr),
        stdout: sanitizeCommandText(options.stdout),
      },
      cause: options.cause,
    });
  }
}

class ParseError extends AppError {
  constructor(message, details) {
    super('PARSE_ERROR', message, { details });
  }
}

class ConflictError extends AppError {
  constructor(message, details) {
    super('CONFLICT', message, { details });
  }
}

class NotFoundError extends AppError {
  constructor(message, details) {
    super('NOT_FOUND', message, { details });
  }
}

function sanitizeCommandText(value) {
  if (!value) return '';
  return String(value).replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g, '').trim().slice(0, 2000);
}

function toPublicError(error) {
  if (error instanceof AppError) {
    return {
      code: error.code,
      message: error.message,
      details: error.details,
      recoverable: error.recoverable,
    };
  }
  return {
    code: 'UNKNOWN_ERROR',
    message: '操作失败，请稍后重试。',
    recoverable: true,
  };
}

module.exports = {
  AppError,
  CommandExecutionError,
  CommandTimeoutError,
  ConflictError,
  NotFoundError,
  ParseError,
  PermissionError,
  ValidationError,
  sanitizeCommandText,
  toPublicError,
};
