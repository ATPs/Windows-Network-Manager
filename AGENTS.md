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
- Create both the Windows NSIS installer and portable ZIP with `npm run dist`.

## Safety

- The frontend may invoke only explicit Tauri commands. Do not expose generic shell, PowerShell, filesystem, or command-execution APIs.
- Keep Windows networking commands in Rust and use fixed PowerShell/netsh argument templates with strict validation.
- Never run real network write operations in automated tests or development validation: no Wi-Fi connect/disconnect, Metric changes, route changes, adapter enable/disable or restart, DHCP renew, IP/DNS/network-category changes, Wi-Fi profile changes, or UAC elevation.
- Preserve the userData configuration location and never store passwords or credentials.
- Coexist with tunnel/proxy software (EasyTier, v2rayN/xray/sing-box/mihomo, Tailscale, ZeroTier, WireGuard): never enable/disable or restart their adapters, renew their DHCP leases, change their IP addresses or DNS, edit their routes or capture routes, or change the system proxy. Only Metric and network category may change, with a warning. Enforce this in Rust, not only in the UI.

## Build cleanup

- After a successful release build, retain the NSIS installer and portable ZIP under `installer/`.
- Remove re-creatable build output before handing off work: `target/`, `dist/`, and `gen/`.
- Do not remove `frontend/node_modules`, because it is required by the npm scripts.
- Remove obsolete temporary files, old runtime artifacts, and unused platform icons when they are not required by the Windows build.
- If a file is locked, report its full path and the cleanup command needed after the locking process exits; do not delete unrelated files.
