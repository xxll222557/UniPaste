# UniPaste

UniPaste 是一个使用 Rust + Tauri 构建的局域网剪贴板同步工具，面向 macOS 与 Windows 双机协作。

![UniPaste App Icon](src-tauri/icons/icon.png)

当前版本已经实现：

- `mDNS` 自动发现局域网设备
- `QUIC` 长连接传输
- 首次配对的 6 位配对码确认
- 配对请求超时自动清理
- 端到端应用层加密
- 本机身份密钥优先存入系统安全存储，并兼容旧版配置迁移
- 文本、HTML、图片、文件剪贴板同步
- 图片以 `PNG` 压缩后传输
- 文件通过独立 QUIC 流分块传输，不再直接塞进剪贴板控制消息
- 去重与回环抑制
- 同步历史记录页面
- 历史记录本地持久化
- 设备名称可在设置中修改
- 可分别控制 HTML / 图片 / 文件同步
- 设置页显示网络状态、监听端口和最近错误
- Windows 平台已接入系统剪贴板更新监听骨架，其他平台保留回退监听

## 开发启动

```bash
npm install
npx tauri dev
```

前端开发端口固定为 `1430`，避免和其他本地项目冲突。

## GitHub 持续集成与发布

仓库里已经包含：

- `.github/workflows/ci.yml`：每次 push / PR 自动执行 `macOS + Windows` 构建检查
- `.github/workflows/release.yml`：在推送 `v*` 标签时自动构建并上传到 GitHub pre-release
- `SHA256SUMS.txt`：release 工作流会自动生成并上传安装包的 SHA256 校验文件

推荐首次上传流程：

```bash
git init
git add .
git commit -m "Initial commit"
git branch -M main
git remote add origin <你的 GitHub 仓库地址>
git pull --no-rebase origin main --allow-unrelated-histories
git push -u origin main
```

发布一个新的预发布版本：

```bash
git tag v0.1.0
git push origin v0.1.0
```

推送标签后，GitHub Actions 会：

- 在 `macos-latest` 上构建 `dmg`
- 在 `windows-latest` 上构建 `nsis exe`
- 自动创建或更新对应版本的 pre-release
- 上传安装包到 release assets
- 额外上传 `SHA256SUMS.txt`

## macOS 调试包

```bash
npx tauri build --debug --bundles app
```

生成的 `.app` 在：

`src-tauri/target/debug/bundle/macos/UniPaste.app`

## Windows 打包

在 Windows 机器上准备：

- Rust `stable-x86_64-pc-windows-msvc`
- Visual Studio Build Tools（含 C++ MSVC 工具链）
- WebView2 Runtime
- Node.js 20+

安装依赖后执行：

```powershell
npm install
npx tauri build --bundles nsis
```

如果你想生成 MSI：

```powershell
npx tauri build --bundles msi
```

产物目录通常位于：

`src-tauri\target\release\bundle\`

## 双机联调

1. 两台机器连接到同一个局域网。
2. 分别启动 UniPaste。
3. 在设备列表里确认双方都能被发现。
4. 在其中一台机器上点“发起配对”。
5. 另一台机器会弹出“配对确认”窗口。
6. 核对两台机器显示的 6 位短码完全一致。
7. 一致则点“确认配对”，不一致则点“拒绝”。
8. 配对完成后，复制文本、富文本、图片或文件，另一台机器应自动收到。
9. 重启应用后，历史页仍会保留最近同步记录。

## 联调建议

- 优先先测文本，再测 HTML、图片，最后测文件。
- 图片同步建议从截图或系统图片复制开始测。
- 文件同步建议先测单个小文件，再测多个文件同时复制。
- 如果你在 Windows 上联调，第一次运行时留意防火墙弹窗和系统剪贴板访问行为。
- 如果发现不了设备，先确认系统防火墙是否拦截了本地 UDP/QUIC 流量。
- 如果配对请求发出但对端没弹窗，先确认两台机器都在设备列表中显示为在线。
- 如果文件没有落到对端，先确认文件大小没有超过 MVP 限制，且复制的是普通文件而不是目录。

## 已知边界

- 当前只做单网络域内同步，不支持跨网段穿透。
- 配对基于应用层身份签名和短码人工确认，不依赖第三方服务器。
- 文件同步当前只支持普通文件，不支持目录和超大文件包。
- 文件落地到对端系统临时目录后再写入剪贴板，属于 MVP 方案。
- 当前只在本机验证了 macOS 构建，Windows 监听与打包链路还需要真机校验。
- 当前历史记录保存在本地配置目录，仅保存最近一段，不做云同步。
