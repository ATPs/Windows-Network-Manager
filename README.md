# Windows Network Manager

Windows Network Manager is a Windows 11 desktop application for inspecting and carefully managing network interfaces. It is built with Tauri 2, Rust, Vite, and Lit.

## What it does

- Lists Wi-Fi, Ethernet, VPN, tunnel, virtual, disconnected, and optionally system interfaces.
- Uses a Windows 11 (Fluent) style interface with Network status, Adapters, Routes, Backups, Diagnostics, Tools, Per-app, Monitor, Hosts file, Recovery, and Settings pages. The sidebar collapses to icons on narrow windows.
- **Network status** shows the active connection (Internet/local-only connectivity, IP, gateway, DNS, Wi-Fi signal), a default-route priority list that explains which adapter Windows prefers and why, clickable counts of changes from Windows defaults, and auto-connect rule status.
- Shows the last successful read-only network state immediately, clearly labels it as cached, then refreshes it in the background.
- Uses per-operation progress, confirmation dialogs with before → after details, toast notifications, and a session **Activity** panel, so slow Windows networking operations never look unresponsive or accept duplicate clicks.
- Supports System, Light, and Dark color themes; the first launch follows Windows and the choice is saved.
- Shows addresses, gateways, DNS servers, DHCP, link speed, interface index, connectivity, network category, connection status, and separate IPv4/IPv6 metrics, with copy buttons. Wi-Fi adapters also show signal, band/channel, security, link rate, and BSSID.
- Lists saved Wi-Fi profiles per wireless adapter and can request a connect or disconnect without reading or storing Wi-Fi passwords. **Connect…** in the Nearby Wi-Fi scan joins a new open, WPA2-Personal or WPA3-Personal network: the password is typed into a dialog, passed in memory to Windows (which saves it in the new profile), wiped afterwards, and never saved, logged or exported by the app.
- Provides opt-in Wi-Fi auto-connect rules per adapter. Each rule uses a saved Windows profile and checks at a configurable interval; if connected to another network, it requests a switch back to the target. Up to three **fallback networks** are tried in order when the preferred one fails; while on a fallback, the rule switches back only when Windows sees the preferred network in range.
- Keeps enabled rules running while the window is closed to the Windows system tray. Use the tray icon to reopen the window or quit the application.
- Displays IPv4 and IPv6 routes with search, family filter, sortable columns, default/app-managed markers, and a metric breakdown (route + interface = effective). Routes are added and edited in a side panel with inline validation.
- Safely previews and applies IPv4/IPv6 route and Metric changes after explicit confirmation.
- Converts eligible temporary static IPv4 routes with **Make persistent** in the Routes table. Conversion briefly replaces the route, preserves its gateway/interface/Metric, and attempts to restore the temporary route on failure. **Undo last app change** restores the previous temporary route.
- Offers **Edit** on eligible manual IPv4 routes. The form fills in the current destination, gateway, interface, route Metric, and persistence; saving previews the changes and records an undo point. Failed edits attempt to restore the original route.
- Shows the application version, default-route candidates, connection freshness, and compact expandable adapter details. Administrators can change interface Metric, add or remove eligible IPv4 routes, and enable or disable eligible non-system adapters after an explicit confirmation.
- Stores one scoped **Undo last app change** recovery point, ten de-duplicated recent snapshots, and named configuration data in the Tauri application-data directory. Restoring a backup previews the actual differences from the current state. Saved configurations and history snapshots can be deleted. **Import…/Export…** on the Backups page save settings, saved configurations and auto-connect rules to a JSON file chosen in a native dialog and read them back after strict validation and a preview; history snapshots, recovery points, app-managed route records and passwords are never exported, and imported auto-connect rules stay disabled until you enable them.
- Offers both **Undo last app change** and **Restore Windows defaults**. Undo keeps its recovery point when one or more steps fail. Windows defaults enables AutomaticMetric only for explicitly selected adapters and removes only routes this application recorded as managed; it does not reset Winsock, reinstall network components, or remove unknown system routes.
- **Tools** offers read-only tests that run only when clicked: a connectivity test that pings each connected adapter's gateway, DNS servers and 1.1.1.1 and resolves `www.msftconnecttest.com`; a route lookup that shows which route, adapter and source address Windows uses for a host, with a cancellable hop-by-hop traceroute; a TCP/UDP connection list with owning processes; and a nearby Wi-Fi scan with signal, channel use and security. The Wi-Fi scan never connects and needs the same Location access as other Wi-Fi details.
- **Monitor** samples locally in the background, without network writes: a live download/upload graph of physical adapters (5 minutes, every 2 seconds) with per-adapter rates, Wi-Fi signal per adapter, latency to the default gateway and local DNS servers, and a persisted history of adapter, Wi-Fi and default-route changes (last 500 events, clearable). Ranges of 1 hour, 24 hours, 7 days and 30 days show **saved history** (one line per minute, downsampled to at most 600 points) with averages, peaks, transferred data, latency p95 and loss, uptime per interface, disconnects, and a daily usage chart and table. Choose interfaces to get a chart for each, switch between Mbps and MB/s, and hide cards you do not need. Tunnel traffic is not added to the total because it is already counted on the physical adapter; tunnel adapters going up or down are hidden and not notified by default; a default route captured by a TUN is recorded as one event.
- **Monitor history** (Settings): on by default, kept for 30 days (1 to 365), about 1 MB per day. Files go to `monitor-data` in the installation folder (next to the program, also for the portable ZIP); if that folder cannot be written, or the program is under Program Files or Windows, the app-data folder is used and Settings says why. **Change folder…** picks a local folder in a Rust dialog (network shares are refused) and copies existing days there without deleting the old ones. Latency uses one 32-byte ping per target every 10, 30 or 60 seconds, only on the physical adapter that carries default traffic, never on tunnel/proxy adapters, and only to private addresses unless you opt in to public DNS servers.
- **Network backups** (Recovery page) save each adapter's IPv4 address or DHCP, DNS servers (fixed or automatic), interface Metric and network category, the routes this app created and the hosts file. Create one with **Create backup now** (optional name); one is also made automatically before every risky change in the app (IP/DNS, category, Metric, routes, domain routes, hosts, restores and Windows defaults). Manual backups are kept until deleted (up to 50); the newest 20 automatic ones are kept and identical ones are not duplicated. **Restore…** lets you untick parts, lists every difference and every skipped item, requires an acknowledgement, saves a backup of the current state first, and is reverted after 60 seconds unless you choose Keep; Undo last app change can also reverse it. If the automatic backup cannot be saved, the change is refused. IP/DNS of adapters owned by tunnel/proxy software are never restored; their Metric and category can be, with a warning. Wi-Fi passwords and profiles, the system proxy, adapter power, IPv6 static addresses and routes made by other programs are not part of a backup.
- **Text size** (Settings, or Ctrl+= / Ctrl+- / Ctrl+0): Small 90%, Default, Large 115% and Extra large 130%. Text and controls are scaled together (webview zoom), so pages keep their layout.
- **Edit IP settings** on an adapter switches IPv4 between DHCP and a manual address/prefix/gateway and sets DNS servers (automatic or up to four manual). **Network category** switches a network between Private and Public. These are the riskiest changes, so each one is checked in Rust (valid unicast address, gateway on the same subnet, manual DNS required with a manual address), warns about specific risks (address already answering on the LAN via a local ARP check, subnet change, losing the default gateway, DHCP without a server, loopback DNS), requires ticking an "I understand" box in the confirmation, records an undo point, and then shows **Keep these settings?** with a 60-second countdown. If you do not choose Keep, the previous settings are restored automatically by the Rust backend, even if the window stops responding; only one such change can be pending. Failed IP changes restore the previous settings immediately. Adapters owned by tunnel/proxy software are refused for IP/DNS changes; their network category can change with a warning.
- **Manage saved networks** on a Wi-Fi adapter reorders saved profiles (priority), switches automatic/manual connection and forgets a profile. Forgetting cannot be undone, is refused while an auto-connect rule uses the profile, and requires typing the profile name; priority and connection-mode changes record an undo point. Profile XML and passwords are never read.
- Routes can be added, edited and removed for **IPv6** as well as IPv4 (with the same capture-route protection for `::/1` and `8000::/1`).
- **Automatic configurations** (Backups page): when a Wi-Fi adapter connects to a chosen saved network, apply a saved configuration's Metric and app-managed routes. They are confirmed once (with an acknowledgement), run only while the app is elevated and never in development builds, never change Wi-Fi or adapter power, run at most once a minute per rule, record an undo point and appear in the Monitor event history.
- **VPN connections** (Network status) lists built-in Windows VPN connections and connects or disconnects them with `rasdial` using the sign-in Windows saved; the app never asks for or stores VPN passwords. **VPN settings** opens the Windows Settings page.
- **Domain routes** (Routes page) route the current IPv4 addresses of a domain (up to 16, as temporary `/32` routes recorded as app-managed) through a chosen gateway, with Refresh and Remove. Lookups that return proxy fake-IP answers (v2rayN, mihomo, sing-box fake-IP mode) are refused, a TUN capturing default traffic is called out, and the previewed addresses are re-checked on apply instead of being resolved again.
- **Hosts file** page edits entries of the fixed Windows hosts file: enable/disable (comment out), edit, add and remove, with comments and blank lines kept unchanged. Saving shows the added/removed entries, requires an acknowledgement and administrator rights, refuses if the file changed since it was loaded, backs up the current file to `hosts-backups` in the app-data folder (last 10 kept) and records an undo point.
- **Per-app rules** (Per-app page) send chosen programs through v2rayN's proxy, direct, blocked, or through another local proxy program (SOCKS or HTTP, e.g. `localhost:1080`). Programs are listed by file name, full path or whole folder (Browse, the running-program list, or typed). Windows has no per-program proxy setting, so the page builds rules for v2rayN 7.x's **Import Rules From Clipboard** plus a `socks://` link for a local proxy, with step-by-step instructions; it writes nothing to v2rayN, the system proxy or Windows. An optional last rule sends everything else direct. **Check port** shows which program listens on a local proxy port and warns when it is v2rayN's own port (which would loop). In system-proxy mode only programs that use the system proxy are matched; v2rayN's TUN mode covers the rest. Folder entries need the Xray core.
- **Quick fixes**: flush the DNS cache (Recovery page), and renew the DHCP lease or restart an adapter (each adapter's Advanced section, administrator only). Each button has a short explanation of what it does and when to use it, and the Recovery page explains each option and suggests trying the least disruptive one first. Each is previewed and confirmed; adapters owned by tunnel/proxy software and system interfaces are refused in Rust.
- Optional **Start with Windows** (Settings): adds the current user's Run entry so the app opens in the tray at sign-in. It is off by default, needs no administrator rights, and is unavailable in development builds.
- Optional Windows notifications for those events (off by default, per category, also while the window is in the tray).
- Coexists with tunnel and proxy software such as **EasyTier** and **v2rayN** (xray, sing-box, mihomo), Tailscale, ZeroTier and WireGuard. Their adapters are labelled "Managed by …" and protected: the app never enables or disables them, never edits or removes their routes or proxy capture routes (`0.0.0.0/1`, `128.0.0.0/1`, `198.18.0.0/15`), leaves them out of Restore Windows defaults, and skips their enabled state when restoring backups. Metric changes on them remain possible with an extra warning. Network status shows detected software, the Windows system proxy (read only) and whether default traffic goes through a tunnel. Backups match adapters by GUID, so an adapter recreated by its owner under a reused index is skipped instead of changed.
- Includes English and Chinese UI languages (including backend warnings), structured read-only diagnostics that can be copied as text or JSON, keyboard shortcuts (Ctrl+1…9, F5, /), a tray tooltip with connection and auto-connect status, and a standard Windows UAC restart-as-administrator option.

The interface is designed for Windows 11 and only invokes structured PowerShell networking cmdlets and `netsh.exe` from Rust. The web frontend receives narrow Tauri commands only; it cannot invoke arbitrary PowerShell, shell commands, Node.js, or filesystem APIs.

## Scope and safety

The app edits an adapter's IPv4 address/gateway, DHCP mode and DNS servers, its network category and saved Wi-Fi profile settings only through the guarded flows described above. It does not edit proxy settings, firewall rules, drivers, credentials, or adapter binding order. It does not implement bandwidth aggregation, load balancing, app-based routing, hotspots, or Internet Connection Sharing.

Network writes can interrupt connectivity. Manual Wi-Fi operations, Metric changes, route changes, undo, and Windows-default restoration require a user action and confirmation. An auto-connect rule is separately confirmed when enabled or changed; it can then request Wi-Fi connections in the background without prompting each time. A saved, enabled rule resumes when the application is opened again. Disable the rule before intentionally using a different Wi-Fi network. Closing the window hides the application in the tray; choose **Quit** in the tray menu to stop it. The app adds itself to Windows startup only when you turn on Start with Windows (a current-user Run entry that opens it in the tray); it never installs a scheduled task or service.

Metric and route operations require an elevated process. Auto-connect is inactive in development builds and automated tests; refresh, diagnostics and Tools never perform network writes. The connectivity test contacts 1.1.1.1 and `www.msftconnecttest.com` only when you run it. In the background the app only pings the local default gateway and private DNS servers for Monitor latency (public DNS servers only if you opt in; this can be turned off in Settings); it never contacts other outside hosts and never changes the Windows system proxy. Neither adapter power nor routes are changed by auto-connect.

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

Configuration is stored as `network-profiles.json` under the Tauri application-data directory, not in the repository. The Monitor event history is stored next to it as `network-events.json` (at most 500 entries), hosts file backups in the `hosts-backups` folder, and network backups in `network-backups.json` with hosts copies in the `network-backups` folder. Monitor history is stored as `YYYY-MM-DD.jsonl` files in `monitor-data` (see Monitor history above); only files with that name pattern are ever deleted. Upgrading keeps the history; uninstalling removes the installation folder's `monitor-data` only when "Delete the application data" is checked. Automatic configurations and domain routes are stored in `network-profiles.json`. The file contains UI preferences, auto-connect rules, named configurations, the last ten unique snapshots, one scoped recovery point, and identifiers of app-managed routes. It never contains Wi-Fi passwords, VPN passwords, or Windows credentials.

Snapshots are normalized and hashed. Saving an identical snapshot moves the existing record to the top instead of creating a duplicate. Restoring a snapshot applies the captured Metric state, the recorded Wi-Fi connection state, and only routes tracked as application-managed; it reports individual success, failure, and skipped results.
