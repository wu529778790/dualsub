# DualSub

> 打开视频，字幕跟上。跨平台桌面播放器：实时语音识别 + 双语字幕，安装包不到 4 MB。

[![build](https://github.com/wu529778790/dualsub/actions/workflows/build.yml/badge.svg)](https://github.com/wu529778790/dualsub/actions/workflows/build.yml)
[![release](https://github.com/wu529778790/dualsub/actions/workflows/release.yml/badge.svg)](https://github.com/wu529778790/dualsub/releases)

## 这是什么

把本地视频（生肉/熟肉/无字幕资源）丢进播放器，画面立即开始播放，whisper 在后台实时生成字幕，翻译随后跟上——**原文 / 译文 / 双语三模式一键切换**。不用去字幕站搜字幕，不用折腾时间轴。

对标 [Transub-Player](https://github.com/dlsandy/Transub-Player)（Windows/WPF），做 Tauri 跨平台版。只参考其架构思路与产品决策，未复用任何代码。

## 当前状态：macOS 可日用（v0.1.0-alpha.1）

| 平台 | 状态 |
|---|---|
| macOS | ✅ 可日用：实时字幕 + 翻译 + 播放控制（需 `brew install mpv`） |
| Windows / Linux | 🔧 编译骨架（播放内核接入中，UI 与翻译管线已就绪） |

**已实现**：libmpv 播放内核（子窗口嵌入）、whisper 实时识别（追帧不追播、静音跳过、幻觉过滤）、在线/本地混合翻译（带缓存）、外挂字幕自动加载（含 GBK 转码）、模型按需下载器（断点续传）。

**未实现**：模型管理页/首启向导（M4）、签名公证与运行时打包（M5）、字幕导出、学习模式。完整规划见 [docs/技术方案.md](docs/技术方案.md)。

## 翻译策略

```
在线免费翻译（默认，秒回、零算力）
   └─ 被内容过滤 / 断网 / 网络受限
        → 自动落回本地模型（全离线、无内容屏蔽）
        → 没装本地模型？设置页一键下载（~4GB，断点续传）
```

安装包只有几 MB——模型不内置，需要时再按需下载。对「在线服务翻不出来的片源」，本地模型就是答案。

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
- [ ] M4 模型管理（设置页 / 首启向导 / 正式翻译模型清单）
- [ ] M5 发布打磨（签名公证 / 运行时打包 / 自动更新）
- [ ] M1-Windows 播放内核接入

## License

暂未确定开源协议（代码与文档当前保留所有权利）。
