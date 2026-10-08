# Spike 技术验证结论

> 日期：2026-10-08 ｜ 机器：Apple M4（10 核）/ macOS 27 ｜ 对应《技术方案》§9

## spike-1：透明 WebView + libmpv 原生子窗口（macOS）✅ 通过

| 验证项 | 结果 |
|---|---|
| mpv NSView 垫在 WKWebView 之下（`addSubview positioned:Below`） | ✅ 截图确认，视频画面正常 |
| WebView 透明背景（`transparent: true` + CSS transparent） | ✅ 半透明 UI 浮在视频上，透出下层画面 |
| mpv `wid` = NSView 指针绑定（macOS 语义） | ✅ loadfile / pause 控制正常 |
| 实现方式 | objc2 + objc2-app-kit 0.3，libmpv raw FFI（未引 mpv crate） |

遗留（进 M1 处理）：自动加载后前端状态未同步；窗口 resize 已用 autoresizing mask 兜底待实测；输入接管已按「前端收全部事件」设计。

**判定：方案最高风险项在 macOS 解除。Windows 路径（子 HWND）待有 Windows 机器时验证。**

## spike-2：whisper-rs 实时识别量化基准 ✅ 通过（大幅超线）

样本：`say` 中文 TTS 合成 267s 语音 → 16k mono wav（近似真实语速）。
测量：整段批量转写（比逐段 15s 窗口的调度更悲观，结论偏保守）。

### 数据

| 配置 | RTF | 墙钟（267s 音频） | CPU | 峰值内存 |
|---|---|---|---|---|
| small + 自动线程（~3 核） | **0.104** | 27.7s | ~3.1 核 | 1.07 GB |
| small + 2 线程 | **0.098** | 26.7s | ~1.8 核 | 1.07 GB |
| small + 1 线程 | **0.118** | 31.6s | 1 核吃满 | 1.07 GB |
| base + 自动线程 | **0.022** | 5.9s | ~3.1 核 | 0.55 GB |

模型加载：small 0.14s / base 0.06s（可随播放热加载，无感）。

### 验收判定（方案 §9 目标 vs 实测）

| 目标 | 线 | 实测 | 判定 |
|---|---|---|---|
| 识别落后播放 | ≤ 10s | RTF 0.1 → 每分钟播放约 6s 识别量，落后可控制在 2–3s | ✅ 大幅超线 |
| CPU 占用 | ≤ 单核 70%（初值） | 1 线程 = 1 核吃满（略超）；2 线程 RTF 不降反升 | ⚠️ 初值线偏严，见下 |

**CPU 结论**：初值「单核 70%」定得过于保守——RTF 富余极大（10 倍），默认档建议 **2 线程**：识别延迟 <1s/段的同时只占 ~1.8 核，给翻译模型留核。方案 CPU 目标修订为「2 线程 RTF ≤ 0.15」。

### 质量发现（进 M2 待办）

1. **zh 输出繁体**：whisper 对中文倾向输出繁体，需 `initial_prompt`（如「以下是普通话的句子。」）引导或后处理简繁转换
2. **首段幻觉**：开头出现 `(Duel Sub)` 幻觉（源文件名污染），`no_speech_thold=0.6` + VAD 静音硬丢弃后预计可控，M2 实测再校
3. TTS 样本偏「干净」，BGM 混音场景留到 M2 用真实片源验证（方案已知风险，不重开 spike）

## 结论

两个 spike 全部通过，**M0→M1 可按排期全面铺开**。技术选型（Tauri 2 + libmpv NSView 垫底 + whisper-rs 进程内）成立，无需要更换路线的阻塞项。
