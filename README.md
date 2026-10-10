# LiveSub-Player

> 打开视频，字幕跟上。跨平台桌面播放器：实时语音识别 + 双语字幕，安装包不到 4 MB。

[![build](https://github.com/wu529778790/livesub-player/actions/workflows/build.yml/badge.svg)](https://github.com/wu529778790/livesub-player/actions/workflows/build.yml)
[![release](https://github.com/wu529778790/livesub-player/actions/workflows/release.yml/badge.svg)](https://github.com/wu529778790/livesub-player/releases)

## 这是什么

把本地视频（生肉/熟肉/无字幕资源）丢进播放器，画面立即开始播放，whisper 在后台实时生成字幕，翻译随后跟上——**原文 / 译文 / 双语三模式一键切换**。不用去字幕站搜字幕，不用折腾时间轴。

## 当前状态：macOS 可日用（v0.1.0-alpha.1）

| 平台 | 状态 |
|---|---|
| macOS | ✅ 可日用：实时字幕 + 翻译 + 播放控制（需 `brew install mpv`） |
| Windows / Linux | 🔧 编译骨架（播放内核接入中，UI 与翻译管线已就绪） |

**已实现**：libmpv 播放内核（子窗口嵌入）、whisper 实时识别（追帧不追播、静音跳过、幻觉过滤）、在线/本地混合翻译（带缓存）、外挂字幕自动加载（含 GBK 转码）、模型按需下载器（断点续传）、应用内自动更新（下载进度条 + 自动重启替换）。

**未实现**：签名公证与运行时打包、字幕导出、学习模式。完整规划见 [docs/技术方案.md](docs/技术方案.md)。

## 翻译策略

```
在线免费翻译（默认，秒回、零算力）
   └─ 被内容过滤 / 断网 / 网络受限
        → 自动落回本地模型（全离线、无内容屏蔽）
        → 没装本地模型？设置页一键下载（~4GB，断点续传）
```

安装包只有几 MB——模型不内置，需要时再按需下载。对「在线服务翻不出来的片源」，本地模型就是答案。

## 安装（macOS，Apple Silicon）

不依赖付费签名公证（个人免费工具，省 $99/年），通过「下载时不打隔离标记」的通道分发，装完双击即开、无 Gatekeeper 拦截：

**方式一：一键脚本（推荐）**

```bash
curl -fsSL https://raw.githubusercontent.com/wu529778790/livesub-player/main/install.sh | bash
```

自动识别最新 Release（含 prerelease）、下载、安装到 /Applications、清理旧版本。指定版本：`| bash -s -- v0.2.0-alpha`。

**方式二：Homebrew**

```bash
brew tap wu529778790/tap https://github.com/wu529778790/homebrew-tap
brew install --cask livesub-player
```

之后 `brew upgrade --cask livesub-player` 跟版本走。cask 定义见 [packaging/cask/livesub-player.rb](packaging/cask/livesub-player.rb)。

**方式三：手动下载**

从 [Releases](https://github.com/wu529778790/livesub-player/releases) 下载 `*-macOS.zip` 解压，拖进「应用程序」。浏览器下载的文件带隔离标记，**首次打开需右键 → 打开 → 再点打开**（只需一次），或终端执行 `xattr -cr /Applications/LiveSub-Player.app`。

## 从源码构建

```bash
# 依赖：Node 18+、Rust stable、cmake、libmpv、ffmpeg
brew install rust cmake mpv ffmpeg   # macOS

npm install
npm run tauri dev     # 开发
npm run tauri build   # 打包
```

Windows 需自行安装 mpv/ffmpeg 并配置链接路径；Linux 需 libwebkit2gtk-4.1 等系统依赖（见 CI 配置）。

## 快捷键

| 按键 | 功能 |
|---|---|
| `空格` | 播放 / 暂停 |
| `←` / `→` | 快退 / 快进 5s |
| `↑` / `↓` | 音量 ±5 |
| `F` | 全屏 |
| `S` | 截图到桌面 |
| `1` / `2` / `3` | 译文 / 原文 / 双语 |

## 架构

```
┌────────────────────────────────────┐
│  WebView（Vue 3，透明，字幕 Overlay）│
│  ────────────────────────────────  │
│  libmpv 原生子窗口（视频垫底）       │
└──────────────┬─────────────────────┘
               │
        Rust 核心（Tauri 2）
        ├─ player  播放控制 + 事件推送
        ├─ asr     ffmpeg 管道抽音频 → VAD → whisper（进程内）
        ├─ mt      在线免费翻译 ↔ llama-server 本地兜底（sidecar）
        └─ models  模型下载器（hf-mirror / 断点续传）
```

识别是软实时：识别领先播放 30s 自动等待，seek 直接丢弃当前窗口重启管线，时间戳以 ffmpeg 输出为基准不漂移。

## 路线图

- [x] M0 脚手架 + 三平台 CI
- [x] M1 播放内核（macOS）
- [x] M2 实时字幕管线
- [x] M3 翻译管线 + 在线/本地混合架构
- [x] M4 模型管理（设置页 / 首启向导 / 混元 MT-7B 本地翻译实测通过）
- [x] M5a 应用内自动更新（Tauri updater + minisign 签名，不依赖 Apple 公证）
- [ ] M5b 发布打磨（签名公证 / 运行时打包）
- [ ] M1-Windows 播放内核接入

## License

暂未确定开源协议（代码与文档当前保留所有权利）。
