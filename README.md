# Windows Network Manager

Windows Network Manager 是一个 Windows 10/11 Electron 桌面工具，用来统一查看和管理 Wi-Fi、有线、VPN、Tunnel 与虚拟网络接口。

## 第一版功能

- 查看全部网络接口、IPv4/IPv6 地址、网关、DNS、DHCP、LinkSpeed 和网络类别。
- 分别查看和设置 IPv4/IPv6 AutomaticMetric 与 InterfaceMetric。
- 管理任意数量的 Wi-Fi 网卡，只使用 Windows 已保存的 Wi-Fi profile，不读取或保存密码。
- 查看、搜索和筛选 IPv4/IPv6 路由，添加 IPv4 临时/持久化路由，精确删除允许删除的静态路由。
- 保存多个网络方案，预览后按顺序应用 Wi-Fi、Metric 和受管理路由，并显示部分成功结果。
- 对明确可操作的网络适配器提供受保护的启用/禁用功能。
- 检测管理员权限，并提供 Windows 标准 UAC“以管理员身份重启”按钮。

本版本不会修改静态 IP、网关、DNS、代理、防火墙、驱动或注册表，也不实现带宽叠加、负载均衡或按应用分流。

## 开发环境

项目使用 CommonJS、Electron 43、原生 HTML/CSS/JavaScript。Node.js 与 npm 位于：

```text
C:\P\miniforge3
```

PowerShell 中安装和启动：

```powershell
& C:\P\miniforge3\npm.cmd install
$env:ELECTRON_RUN_AS_NODE = $null
& C:\P\miniforge3\npm.cmd start
```

CMD 中可以先执行：

```cmd
set "PATH=C:\P\miniforge3;%PATH%"
set ELECTRON_RUN_AS_NODE=
npm start
```

## 测试与检查

```powershell
& C:\P\miniforge3\npm.cmd run check
& C:\P\miniforge3\npm.cmd test
```

测试使用 `node:test`、fixture 和 mock。自动测试不会连接/断开 Wi-Fi、修改 Metric、增删路由、禁用网卡或触发 UAC。

## 管理员权限与安全警告

普通权限可以查看全部状态、操作 Wi-Fi 和管理应用内方案。以下操作需要管理员权限：

- 修改 IPv4/IPv6 Metric。
- 添加或删除路由。
- 启用或禁用网络适配器。

点击顶部“以管理员身份重启”后，Windows 会显示标准 UAC 提示。应用不绕过 UAC，也不保存管理员凭据。

修改默认出口 Metric、默认路由或禁用承载当前网络的接口可能导致断网。应用会先生成预览和警告，并要求明确确认，但无法可靠判断所有远程连接依赖关系。

## 用户配置

网络方案、界面设置和路由备注保存在 Electron `app.getPath("userData")` 下的 `network-profiles.json`，不会写入源码目录，也不会打入安装包。配置损坏时，应用会在同一 userData 目录备份损坏文件并以空配置启动。

## 打包

```powershell
& C:\P\miniforge3\npm.cmd run dist
```

NSIS 安装程序和未打包版本生成到 `release`。应用名称为 `Windows Network Manager`，不强制管理员启动，当前使用默认 Electron 图标且不进行代码签名，因此 Windows SmartScreen 可能显示提示。

## 依赖说明

- `electron`：桌面运行环境。
- `iconv-lite`：兼容 UTF-8、GBK/CP936 等 Windows `netsh` 输出编码。
- `electron-builder`：构建 Windows NSIS 安装程序。
