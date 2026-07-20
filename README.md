# Windows Network Manager

Windows Network Manager is a Windows 11 desktop application for inspecting and carefully managing network interfaces. It is built with Tauri 2, Rust, Vite, and plain HTML/CSS/JavaScript.

## What it does

- Lists Wi-Fi, Ethernet, VPN, tunnel, virtual, disconnected, and optionally system interfaces.
- Shows addresses, gateways, DNS servers, DHCP, link speed, interface index, connection status, and separate IPv4/IPv6 metrics.
- Lists saved Wi-Fi profiles per wireless adapter and can request a connect or disconnect without reading, storing, or creating Wi-Fi passwords.
- Displays IPv4 and IPv6 routes with filtering and a concise summary of manual metrics, static routes, and disabled adapters.
- Safely previews and applies IPv4 route and Metric changes after explicit confirmation.
- Stores a pre-change baseline, ten de-duplicated recent snapshots, named configuration data, and JSON import/export data in the Tauri application-data directory.
- Offers both **Reset to pre-change baseline** and **Restore Windows defaults**. Windows defaults enables AutomaticMetric and removes only routes this application recorded as managed; it does not reset Winsock, reinstall network components, or remove unknown system routes.
- Includes English and Chinese UI languages, safe diagnostics, and a standard Windows UAC restart-as-administrator option.

The interface is designed for Windows 11 and only invokes structured PowerShell networking cmdlets and `netsh.exe` from Rust. The web frontend receives narrow Tauri commands only; it cannot invoke arbitrary PowerShell, shell commands, Node.js, or filesystem APIs.

## Scope and safety

The first version does not edit static IP addresses, gateway addresses, DNS, proxy settings, firewall rules, drivers, credentials, or adapter binding order. It does not implement bandwidth aggregation, load balancing, app-based routing, hotspots, or Internet Connection Sharing.

Network writes can interrupt connectivity. Wi-Fi operations, Metric changes, route edits, baseline restoration, and Windows-default restoration always require a user action and confirmation. Metric and route operations also require an elevated process. The application never automatically performs network writes during startup, refresh, diagnostics, development, or tests.

The UAC button restarts the application using the normal Windows elevation prompt. It does not bypass UAC or store administrator credentials.

## Prerequisites

- Windows 11 x64
- Node.js (current LTS or newer)
- Rust stable with the MSVC target
- Visual Studio Build Tools with the **Desktop development with C++** / MSVC build-tools workload
- Microsoft Edge WebView2 Evergreen Runtime

## Development

Install the frontend dependencies once:

```powershell
cd frontend
npm install
cd ..
```

Then run:

```powershell
npm start
```

Useful checks:

```powershell
npm run check
npm test
npm run build
```

`npm test` runs Rust unit tests only. No test invokes a Wi-Fi connection, changes Metric, adds/removes a route, disables an adapter, or triggers UAC.

## Packaging

Build the NSIS installer with:

```powershell
npm run dist
```

The installer is per-user, does not require an elevated installation, and uses the bundled Tauri configuration. User configurations remain in the Windows application-data directory and are never packaged with the application. Code signing is not configured, so Windows may show a SmartScreen warning.

## Stored data and recovery

Configuration is stored as `network-profiles.json` under the Tauri application-data directory, not in the repository. The file contains UI preferences, named configurations, the last ten unique snapshots, one pre-change baseline, and identifiers of app-managed routes. It never contains Wi-Fi passwords, VPN passwords, or Windows credentials.

Snapshots are normalized and hashed. Saving an identical snapshot moves the existing record to the top instead of creating a duplicate. Restoring a snapshot applies the captured Metric state, the recorded Wi-Fi connection state, and only routes tracked as application-managed; it reports individual success, failure, and skipped results.
