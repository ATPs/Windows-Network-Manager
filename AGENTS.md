# Development Guide

## Project shape

- This is a Windows-only Tauri 2 project. Do not add Electron, Node.js backend code, or Electron build tooling.
- Keep the Tauri project flat at the repository root:
  - `Cargo.toml`, `Cargo.lock`, `build.rs`, and `tauri.conf.json`
  - Rust code in `src/`
  - Tauri permissions in `capabilities/`
  - Windows icon at `icons/icon.ico`
  - Vite frontend in `frontend/`
- Do not recreate a `src-tauri/` directory.

## Commands

- Install frontend dependencies once with `cd frontend; npm install`.
- Run the application with `npm start`.
- Validate with `npm run check`, `npm test`, and `npm run build`.
- Create the Windows NSIS installer with `npm run dist`.

## Safety

- The frontend may invoke only explicit Tauri commands. Do not expose generic shell, PowerShell, filesystem, or command-execution APIs.
- Keep Windows networking commands in Rust and use fixed PowerShell/netsh argument templates with strict validation.
- Never run real network write operations in automated tests or development validation: no Wi-Fi connect/disconnect, Metric changes, route changes, adapter enable/disable, or UAC elevation.
- Preserve the userData configuration location and never store passwords or credentials.

## Build cleanup

- After a successful build, retain the user-facing NSIS installer under `installer/`.
- Remove re-creatable build output before handing off work: `target/`, `dist/`, and `gen/`.
- Do not remove `frontend/node_modules`, because it is required by the npm scripts.
- Remove obsolete temporary files, old runtime artifacts, and unused platform icons when they are not required by the Windows build.
- If a file is locked, report its full path and the cleanup command needed after the locking process exits; do not delete unrelated files.
