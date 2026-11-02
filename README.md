# Windows Network Manager

Windows Network Manager is a Windows 11 desktop application for inspecting and carefully managing network interfaces. It is built with Tauri 2, Rust, Vite, and Lit.

## What it does

- Lists Wi-Fi, Ethernet, VPN, tunnel, virtual, disconnected, and optionally system interfaces.
- Uses a Windows 11 (Fluent) style interface with Network status, Adapters, Routes, Backups, Diagnostics, Tools, Monitor, Recovery, and Settings pages. The sidebar collapses to icons on narrow windows.
- **Network status** shows the active connection (Internet/local-only connectivity, IP, gateway, DNS, Wi-Fi signal), a default-route priority list that explains which adapter Windows prefers and why, clickable counts of changes from Windows defaults, and auto-connect rule status.
- Shows the last successful read-only network state immediately, clearly labels it as cached, then refreshes it in the background.
- Uses per-operation progress, confirmation dialogs with before → after details, toast notifications, and a session **Activity** panel, so slow Windows networking operations never look unresponsive or accept duplicate clicks.
- Supports System, Light, and Dark color themes; the first launch follows Windows and the choice is saved.
- Shows addresses, gateways, DNS servers, DHCP, link speed, interface index, connectivity, network category, connection status, and separate IPv4/IPv6 metrics, with copy buttons. Wi-Fi adapters also show signal, band/channel, security, link rate, and BSSID.
- Lists saved Wi-Fi profiles per wireless adapter and can request a connect or disconnect without reading, storing, or creating Wi-Fi passwords.
- Provides opt-in Wi-Fi auto-connect rules per adapter. Each rule uses a saved Windows profile and checks at a configurable interval; if connected to another network, it requests a switch back to the target.
- Keeps enabled rules running while the window is closed to the Windows system tray. Use the tray icon to reopen the window or quit the application.
- Displays IPv4 and IPv6 routes with search, family filter, sortable columns, default/app-managed markers, and a metric breakdown (route + interface = effective). Routes are added and edited in a side panel with inline validation.
- Safely previews and applies IPv4/IPv6 route and Metric changes after explicit confirmation.
- Converts eligible temporary static IPv4 routes with **Make persistent** in the Routes table. Conversion briefly replaces the route, preserves its gateway/interface/Metric, and attempts to restore the temporary route on failure. **Undo last app change** restores the previous temporary route.
- Offers **Edit** on eligible manual IPv4 routes. The form fills in the current destination, gateway, interface, route Metric, and persistence; saving previews the changes and records an undo point. Failed edits attempt to restore the original route.
- Shows the application version, default-route candidates, connection freshness, and compact expandable adapter details. Administrators can change interface Metric, add or remove eligible IPv4 routes, and enable or disable eligible non-system adapters after an explicit confirmation.
- Stores one scoped **Undo last app change** recovery point, ten de-duplicated recent snapshots, and named configuration data in the Tauri application-data directory. Restoring a backup previews the actual differences from the current state. Saved configurations and history snapshots can be deleted. **Import…/Export…** on the Backups page save settings, saved configurations and auto-connect rules to a JSON file chosen in a native dialog and read them back after strict validation and a preview; history snapshots, recovery points, app-managed route records and passwords are never exported, and imported auto-connect rules stay disabled until you enable them.
- Offers both **Undo last app change** and **Restore Windows defaults**. Undo keeps its recovery point when one or more steps fail. Windows defaults enables AutomaticMetric only for explicitly selected adapters and removes only routes this application recorded as managed; it does not reset Winsock, reinstall network components, or remove unknown system routes.
- **Tools** offers read-only tests that run only when clicked: a connectivity test that pings each connected adapter's gateway, DNS servers and 1.1.1.1 and resolves `www.msftconnecttest.com`; a route lookup that shows which route, adapter and source address Windows uses for a host, with a cancellable hop-by-hop traceroute; a TCP/UDP connection list with owning processes; and a nearby Wi-Fi scan with signal, channel use and security. The Wi-Fi scan never connects and needs the same Location access as other Wi-Fi details.
- **Monitor** samples locally in the background, without network writes or outside hosts: a live download/upload graph of physical adapters (5 minutes, every 2 seconds) with per-adapter rates, a Wi-Fi signal history per adapter (1 hour, every 10 seconds), and a persisted history of adapter, Wi-Fi and default-route changes (last 500 events, clearable). Tunnel traffic is not added to the total because it is already counted on the physical adapter; tunnel adapters going up or down are hidden and not notified by default; a default route captured by a TUN is recorded as one event.
- **Edit IP settings** on an adapter switches IPv4 between DHCP and a manual address/prefix/gateway and sets DNS servers (automatic or up to four manual). **Network category** switches a network between Private and Public. These are the riskiest changes, so each one is checked in Rust (valid unicast address, gateway on the same subnet, manual DNS required with a manual address), warns about specific risks (address already answering on the LAN via a local ARP check, subnet change, losing the default gateway, DHCP without a server, loopback DNS), requires ticking an "I understand" box in the confirmation, records an undo point, and then shows **Keep these settings?** with a 60-second countdown. If you do not choose Keep, the previous settings are restored automatically by the Rust backend, even if the window stops responding; only one such change can be pending. Failed IP changes restore the previous settings immediately. Adapters owned by tunnel/proxy software are refused for IP/DNS changes; their network category can change with a warning.
- **Manage saved networks** on a Wi-Fi adapter reorders saved profiles (priority), switches automatic/manual connection and forgets a profile. Forgetting cannot be undone, is refused while an auto-connect rule uses the profile, and requires typing the profile name; priority and connection-mode changes record an undo point. Profile XML and passwords are never read.
- Routes can be added, edited and removed for **IPv6** as well as IPv4 (with the same capture-route protection for `::/1` and `8000::/1`).
- **Quick fixes**: flush the DNS cache (Recovery page), and renew the DHCP lease or restart an adapter (each adapter's Advanced section, administrator only). Each is previewed and confirmed; adapters owned by tunnel/proxy software and system interfaces are refused in Rust.
- Optional **Start with Windows** (Settings): adds the current user's Run entry so the app opens in the tray at sign-in. It is off by default, needs no administrator rights, and is unavailable in development builds.
- Optional Windows notifications for those events (off by default, per category, also while the window is in the tray).
- Coexists with tunnel and proxy software such as **EasyTier** and **v2rayN** (xray, sing-box, mihomo), Tailscale, ZeroTier and WireGuard. Their adapters are labelled "Managed by …" and protected: the app never enables or disables them, never edits or removes their routes or proxy capture routes (`0.0.0.0/1`, `128.0.0.0/1`, `198.18.0.0/15`), leaves them out of Restore Windows defaults, and skips their enabled state when restoring backups. Metric changes on them remain possible with an extra warning. Network status shows detected software, the Windows system proxy (read only) and whether default traffic goes through a tunnel. Backups match adapters by GUID, so an adapter recreated by its owner under a reused index is skipped instead of changed.
- Includes English and Chinese UI languages (including backend warnings), structured read-only diagnostics that can be copied as text or JSON, keyboard shortcuts (Ctrl+1…9, F5, /), a tray tooltip with connection and auto-connect status, and a standard Windows UAC restart-as-administrator option.

The interface is designed for Windows 11 and only invokes structured PowerShell networking cmdlets and `netsh.exe` from Rust. The web frontend receives narrow Tauri commands only; it cannot invoke arbitrary PowerShell, shell commands, Node.js, or filesystem APIs.

## Scope and safety

The app edits an adapter's IPv4 address/gateway, DHCP mode and DNS servers, its network category and saved Wi-Fi profile settings only through the guarded flows described above. It does not edit proxy settings, firewall rules, drivers, credentials, or adapter binding order. It does not implement bandwidth aggregation, load balancing, app-based routing, hotspots, or Internet Connection Sharing.

Network writes can interrupt connectivity. Manual Wi-Fi operations, Metric changes, route changes, undo, and Windows-default restoration require a user action and confirmation. An auto-connect rule is separately confirmed when enabled or changed; it can then request Wi-Fi connections in the background without prompting each time. A saved, enabled rule resumes when the application is opened again. Disable the rule before intentionally using a different Wi-Fi network. Closing the window hides the application in the tray; choose **Quit** in the tray menu to stop it. The app adds itself to Windows startup only when you turn on Start with Windows (a current-user Run entry that opens it in the tray); it never installs a scheduled task or service.

Metric and route operations require an elevated process. Auto-connect is inactive in development builds and automated tests; refresh, diagnostics and Tools never perform network writes. The connectivity test contacts 1.1.1.1 and `www.msftconnecttest.com` only when you run it; the app never contacts outside hosts in the background and never changes the Windows system proxy. Neither adapter power nor routes are changed by auto-connect.

On current Windows versions, Wi-Fi connection details require location-related Wi-Fi access. The app checks that access without requesting it or showing a background permission dialog. If access is unavailable, the rule waits and displays a message directing you to the Windows Location settings.

If the earlier standalone `WiFiWatchdog` scheduled task is still enabled, disable it in Task Scheduler when replacing it with an app rule to avoid competing reconnect attempts. The app does not modify or remove that task.

Quit any older running version from its window or tray before starting the new version, so the older process cannot compete with the new auto-connect worker.

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
npm run test:frontend
npm run build
```

`npm test` runs Rust unit tests and `npm run test:frontend` runs frontend unit tests. No test invokes a Wi-Fi connection, changes Metric, adds/removes a route, disables an adapter, or triggers UAC.

## Packaging

Build the NSIS installer and portable ZIP with:

```powershell
npm run dist
```

The installer is per-user and does not require an elevated installation. Installing under the current user's profile does not prevent the application from requesting administrator privileges later through standard Windows UAC. The portable ZIP can be extracted anywhere and run directly. Both packages use the system WebView2 runtime.

User configurations remain in the Windows application-data directory and are never packaged with the application. Code signing is not configured, so Windows may show a SmartScreen warning. The release script removes re-creatable `target`, `dist`, and `gen` output after copying both packages to `installer/`.

## Stored data and recovery

Configuration is stored as `network-profiles.json` under the Tauri application-data directory, not in the repository. The Monitor event history is stored next to it as `network-events.json` (at most 500 entries). The file contains UI preferences, auto-connect rules, named configurations, the last ten unique snapshots, one scoped recovery point, and identifiers of app-managed routes. It never contains Wi-Fi passwords, VPN passwords, or Windows credentials.

Snapshots are normalized and hashed. Saving an identical snapshot moves the existing record to the top instead of creating a duplicate. Restoring a snapshot applies the captured Metric state, the recorded Wi-Fi connection state, and only routes tracked as application-managed; it reports individual success, failure, and skipped results.
