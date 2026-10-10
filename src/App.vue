<template>
  <div class="shell" :class="{ 'ui-hidden': uiHidden }" @mousemove="onUiActivity">
    <header class="topbar">
      <span class="filename">{{ filename }}</span>
      <button class="bar-btn" @click="openFile">打开</button>
      <button class="bar-btn" @click="settingsOpen = true">设置</button>
    </header>

    <!-- 设置抽屉：右侧滑出，点遮罩或 ✕ 关闭 -->
    <transition name="fade">
      <div v-if="settingsOpen" class="drawer-mask" @click="settingsOpen = false" />
    </transition>
    <div class="drawer" :class="{ open: settingsOpen }">
      <div class="drawer-head">
        <span>设置</span>
        <button class="drawer-close" title="关闭" @click="settingsOpen = false">✕</button>
      </div>

      <div class="drawer-section">常规</div>
      <label class="set-row">
        <span>翻译引擎</span>
        <select :value="engine" @change="changeEngine">
          <option value="auto">自动（优先在线，失败用本地）</option>
          <option value="online">仅在线翻译</option>
          <option value="local">仅本地模型（离线）</option>
        </select>
      </label>
      <label class="set-row">
        <span>语音识别</span>
        <select :value="whisperModel" @change="changeWhisperModel">
          <option value="small">均衡（默认，推荐）</option>
          <option value="base">快速（电脑配置较低时）</option>
        </select>
      </label>

      <div class="drawer-section">模型管理</div>
      <div v-for="m in modelStatus" :key="m.name" class="model-row">
        <span class="model-desc">{{ m.desc }}</span>
        <span v-if="m.installed" class="model-ok">已装 {{ m.sizeMb }}MB</span>
        <template v-else>
          <button class="btn small" :disabled="downloadingName === m.name" @click="download(m.name)">
            {{ downloadingName === m.name ? `下载中 ${downloadPct}%` : "下载" }}
          </button>
        </template>
        <button
          v-if="m.installed"
          class="btn small ghost"
          @click="removeModel(m.name)"
        >{{ pendingDelete === m.name ? "确认删除" : "删除" }}</button>
      </div>
      <p class="set-hint">
        在线翻译需要能访问 Google（自动走系统代理）；本地翻译完全离线，没有内容限制。
      </p>
    </div>

    <!-- 视频区域：播放中背景全透明，透出下层 mpv 画面；未加载时给不透明底色防空洞透桌；输入由前端接管 -->
    <main
      class="stage"
      :class="{ 'stage-empty': !loaded }"
      @click="onStageClick"
      @dblclick="toggleFullscreen"
      @contextmenu.prevent="openCtx"
      @wheel.prevent="onWheel"
      @dragover.prevent
      @drop.prevent="onDrop"
    >
      <div v-if="!loaded" class="panel">
        <template v-if="showWizard">
          <p class="wizard-title">欢迎使用 LiveSub-Player</p>
          <p class="wizard-text">
            语音识别需要下载识别模型（~466MB）；<br />
            在线翻译开箱即用；本地翻译模型（~4.4GB）可离线翻译、无视内容屏蔽。
          </p>
          <div class="wizard-actions">
            <button class="btn" @click.stop="wizardDownloadAll">
              {{ wizardDownloading ? "下载中…" : "下载模型（识别+本地翻译）" }}
            </button>
            <button class="btn ghost" @click.stop="wizardDismiss">仅在线翻译，直接开始</button>
          </div>
          <p class="status">{{ status }}</p>
        </template>
        <template v-else>
          <p>把视频文件拖进窗口，或点「打开」</p>
          <p class="status">{{ status }}</p>
        </template>
      </div>

      <!-- 实时字幕 Overlay（技术方案决策 B：DOM 渲染，不用 mpv sub 轨） -->
      <div v-if="activeSub" class="subtitle-overlay">
        <div v-if="mode !== '译文'" class="sub-src">{{ activeSub.text }}</div>
        <div v-if="mode !== '原文' && activeTranslation" class="sub-dst">{{ activeTranslation }}</div>
        <div
          v-else-if="mode === '译文' && mtAvailable"
          class="sub-dst pending"
        >译文生成中…</div>
      </div>

      <!-- 识别进度指示（方案 §2.2：进度可见，绝不让人以为卡死） -->
      <div v-if="recognizing && recognizing.start >= timePos" class="asr-progress">
        正在识别 {{ fmtTime(recognizing.start) }} – {{ fmtTime(recognizing.end) }}
      </div>
      <div v-if="asrError" class="asr-error">{{ asrError }}</div>

      <!-- 在线翻译失败 → 引导下载本地模型（产品决策：告知 + 兜底） -->
      <div v-if="needLocalModel" class="cta-bar">
        <span>在线翻译不可用（可能被过滤或网络受限）。本地模型可离线翻译、无视屏蔽。</span>
        <button class="btn small" @click.stop="downloadLocalModel">
          {{ downloading ? `下载中 ${downloadPct}%` : "下载本地模型" }}
        </button>
      </div>

      <!-- 新版本提示：应用内自动更新（下载完成后自动重启替换） -->
      <div v-if="updateInfo?.hasUpdate" class="update-bar">
        <span>新版本 v{{ updateInfo.latest }} 可用（当前 v{{ updateInfo.current }}）</span>
        <button v-if="updatePct === null" class="btn small" @click.stop="installUpdate">立即更新</button>
        <button v-else class="btn small" disabled>更新中 {{ updatePct }}%</button>
      </div>

      <!-- 操作反馈 OSD（截图/快进快退等，2.5s 自动消失） -->
      <div v-if="toast" class="osd">{{ toast }}</div>
    </main>

    <!-- 控制栏 -->
    <footer v-if="loaded" class="controls">
      <button class="ctl-btn" @click="togglePause">{{ paused ? "▶" : "⏸" }}</button>
      <button class="ctl-btn" title="后退 10s（←）" @click="seekBy(-10)">⏪</button>
      <span class="time">{{ fmtTime(displayTime) }}</span>
      <div
        ref="barRef"
        class="progress"
        @pointerdown="startDrag"
        @pointermove="onBarMove"
        @pointerup="endDrag"
        @pointercancel="endDrag"
        @pointerenter="hovering = true"
        @pointerleave="onBarLeave"
      >
        <div class="progress-fill" :style="{ width: progressPct }" />
        <div class="progress-thumb" :style="{ left: progressPct }" />
        <div v-if="hovering" class="progress-tip" :style="{ left: hoverPct }">
          {{ fmtTime(hoverTime) }}
        </div>
      </div>
      <span class="time">/ {{ fmtTime(duration) }}</span>
      <div class="speed-wrap">
        <button class="speed-btn" title="倍速（滚轮+Shift 微调，[ ] 快捷键）" @click.stop="speedPopOpen = !speedPopOpen">
          {{ fmtSpeed(speed) }}
        </button>
        <div v-if="speedPopOpen" class="speed-pop" @click.stop>
          <div class="speed-grid">
            <button
              v-for="s in SPEED_PRESETS"
              :key="s"
              class="speed-chip"
              :class="{ on: speed === s }"
              @click="setSpeed(s)"
            >{{ fmtSpeed(s) }}</button>
          </div>
          <div class="speed-grid">
            <button class="speed-chip" @click="setSpeed(speed - 0.1)">−0.1</button>
            <button class="speed-chip" @click="setSpeed(1)">重置</button>
            <button class="speed-chip" @click="setSpeed(speed + 0.1)">+0.1</button>
          </div>
        </div>
      </div>
      <span class="vol-label">音量</span>
      <input
        class="vol"
        type="range"
        min="0"
        max="100"
        :value="volume"
        @input="changeVolume"
      />
      <span class="vol-num">{{ Math.round(volume) }}</span>
      <button class="ctl-btn" title="截图到桌面（S）" @click="screenshot">📷</button>
      <button class="ctl-btn" @click="toggleFullscreen">⛶</button>
    </footer>

    <!-- 右键菜单 -->
    <div v-if="ctxOpen" class="ctx-mask" @click="ctxOpen = false" @contextmenu.prevent="ctxOpen = false" />
    <div v-if="ctxOpen" class="ctx-menu" :style="{ left: ctxX + 'px', top: ctxY + 'px' }">
      <button class="ctx-item" @click="ctxOpen = false; openFile()">打开文件…</button>
      <button class="ctx-item" @click="ctxOpen = false; togglePause()">{{ paused ? "播放" : "暂停" }}</button>
      <button class="ctx-item" @click="ctxOpen = false; screenshot()">截图</button>
      <button class="ctx-item" @click="ctxOpen = false; toggleFullscreen()">全屏</button>
      <div class="ctx-divider" />
      <div class="ctx-label">字幕模式</div>
      <button
        v-for="m in ['双语', '译文', '原文']"
        :key="m"
        class="ctx-item"
        :class="{ on: mode === m }"
        @click="mode = m; ctxOpen = false"
      >{{ mode === m ? "✓ " : "" }}{{ m }}</button>
      <div class="ctx-divider" />
      <div class="ctx-label">倍速</div>
      <div class="ctx-speeds">
        <button
          v-for="s in SPEED_PRESETS"
          :key="s"
          class="ctx-speed"
          :class="{ on: speed === s }"
          @click="setSpeed(s); ctxOpen = false"
        >{{ fmtSpeed(s) }}</button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open } from "@tauri-apps/plugin-dialog";

const VIDEO_EXTS = ["mp4", "mkv", "avi", "mov", "webm", "m4v", "ts", "flv", "wmv", "rmvb"];

const SPEED_PRESETS = [0.5, 0.75, 1, 1.25, 1.5, 2];

const filename = ref("");
const status = ref("");
const loaded = ref(false);
const paused = ref(false);
const timePos = ref(0);
const duration = ref(0);
const volume = ref(100);
const speed = ref(1);
const displayTime = ref(0); // 拖动进度条时的临时显示值
const dragging = ref(false);

// 实时字幕状态（asr:// 事件）
interface Sub {
  start: number;
  end: number;
  text: string;
}
const subs = ref<Sub[]>([]);
const translations = ref<{ start: number; end: number; src: string; dst: string }[]>([]);
const mtAvailable = ref(true);
const settingsOpen = ref(false);
const speedPopOpen = ref(false);
// 右键菜单
const ctxOpen = ref(false);
const ctxX = ref(0);
const ctxY = ref(0);
// 模型删除二次确认（3s 内再点一次才真正删）
const pendingDelete = ref("");
let pendingDeleteTimer: number | undefined;
// 播放中鼠标静止自动隐藏顶栏/控制栏
const idle = ref(false);
let idleTimer: number | undefined;
const engine = ref("auto");
const needLocalModel = ref(false);
const downloading = ref(false);
const downloadPct = ref(0);

// 模型管理与首启向导
interface ModelStatus {
  name: string;
  desc: string;
  installed: boolean;
  sizeMb: number;
}
const modelStatus = ref<ModelStatus[]>([]);
const downloadingName = ref("");
const whisperModel = ref("small");
const showWizard = ref(false);
const wizardDownloading = ref(false);

// 新版本检查（应用内自动更新）
const updateInfo = ref<{ current: string; latest: string; hasUpdate: boolean; notes?: string | null } | null>(null);
const updatePct = ref<number | null>(null); // null=未在更新中
const recognizing = ref<{ start: number; end: number } | null>(null);
const asrError = ref("");
const mode = ref<"译文" | "原文" | "双语">("双语");

const activeSub = computed(() => {
  for (let i = subs.value.length - 1; i >= 0; i--) {
    const s = subs.value[i];
    if (s.start <= timePos.value && timePos.value <= s.end) return s;
  }
  return null;
});
// 译文：优先时间重叠，其次按源文本匹配（缓存命中时时间可能略有出入）
const activeTranslation = computed(() => {
  const ts = translations.value;
  for (let i = ts.length - 1; i >= 0; i--) {
    const t = ts[i];
    if (t.start <= timePos.value && timePos.value <= t.end) return t.dst;
  }
  const src = activeSub.value?.text;
  if (src) {
    for (let i = ts.length - 1; i >= 0; i--) {
      if (ts[i].src === src) return ts[i].dst;
    }
  }
  return "";
});

const barRef = ref<HTMLElement | null>(null);

// 进度条悬停预览
const hovering = ref(false);
const hoverTime = ref(0);
const hoverPct = computed(
  () => `${(hoverTime.value / (duration.value || 1)) * 100}%`
);

// OSD 操作反馈
const toast = ref("");
let toastTimer: number | undefined;
function showToast(msg: string) {
  toast.value = msg;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (toast.value = ""), 2500);
}

let unlisten: UnlistenFn | null = null;

const progressPct = computed(() => {
  const d = duration.value || 1;
  return `${Math.min(100, ((dragging.value ? displayTime.value : timePos.value) / d) * 100)}%`;
});

// ---- mpv 状态事件（Rust 属性观察推送） ----
onMounted(async () => {
  unlisten = await listen<{
    timePos: number | null;
    duration: number | null;
    paused: boolean | null;
  }>("player://state", (e) => {
    const p = e.payload;
    if (p.duration != null) {
      duration.value = p.duration;
      loaded.value = loaded.value || p.duration > 0;
    }
    if (p.timePos != null && !dragging.value) {
      timePos.value = p.timePos;
      displayTime.value = p.timePos;
    }
    if (p.paused != null) paused.value = p.paused;
  });
  // 初始音量/倍速
  const st = await invoke<{ timePos: number | null; duration: number | null; paused: boolean | null }>("cmd_player_get_state").catch(() => null);
  if (st?.duration) {
    loaded.value = true;
    duration.value = st.duration;
    if (st.timePos != null) timePos.value = st.timePos;
    if (st.paused != null) paused.value = st.paused;
  }
  window.addEventListener("keydown", onKey);
  // asr 事件
  await listen<Sub>("asr://subtitle", (e) => {
    subs.value.push(e.payload);
  });
  await listen<{ start: number; end: number }>("asr://progress", (e) => {
    recognizing.value = e.payload;
  });
  await listen<string>("asr://error", (e) => {
    asrError.value = String(e.payload);
  });
  await listen<{
    start: number;
    end: number;
    src: string;
    dst: string;
  }>("asr://translation", (e) => {
    translations.value.push(e.payload);
  });
  await listen<string>("mt://unavailable", () => {
    mtAvailable.value = false;
  });
  await listen<string>("mt://need-local-model", (e) => {
    needLocalModel.value = true;
    status.value = String(e.payload);
  });
  await listen<{ name: string; downloadedMb: number; totalMb: number }>(
    "models://progress",
    (e) => {
      downloading.value = true;
      downloadingName.value = e.payload.name;
      downloadPct.value = Math.min(
        100,
        Math.round((e.payload.downloadedMb / (e.payload.totalMb || 1)) * 100)
      );
    }
  );
  await listen<{ name: string }>("models://done", async (e) => {
    downloading.value = false;
    downloadingName.value = "";
    needLocalModel.value = false;
    status.value = "模型下载完成";
    await refreshModelStatus();
    // whisper 模型装好后，下次识别会话自动使用
  });
  await listen<string>("models://error", (e) => {
    downloading.value = false;
    downloadingName.value = "";
    status.value = `模型下载失败: ${e.payload}`;
  });
  engine.value = await invoke<string>("cmd_mt_get_engine");
  whisperModel.value = (await invoke<string>("cmd_get_setting", { key: "whisper_model" })) || "small";
  await refreshModelStatus();
  // 首启向导：识别模型未装且未跳过过
  showWizard.value =
    !modelStatus.value.find((m) => m.name === "whisper-small")?.installed &&
    (await invoke<string>("cmd_get_setting", { key: "wizard_done" })) !== "1";
  // 新版本检查（静默失败即可）
  updateInfo.value = await invoke("cmd_check_update").catch(() => null);
  await listen<number>("update://progress", (e) => {
    updatePct.value = e.payload;
  });
});

async function installUpdate() {
  if (updatePct.value !== null) return;
  updatePct.value = 0;
  const ok = await invoke("cmd_install_update").catch((e) => {
    status.value = String(e);
    return false;
  });
  if (!ok && updatePct.value !== null && updatePct.value < 100) updatePct.value = null;
  // 成功路径：安装完成后应用自动重启替换，无需收尾
}

async function refreshModelStatus() {
  modelStatus.value = await invoke<ModelStatus[]>("cmd_model_status");
}

async function download(name: string) {
  if (downloading.value) return;
  await invoke("cmd_model_download", { name }).catch((e) => (status.value = String(e)));
}

async function removeModel(name: string) {
  // 二次确认：3s 内再点一次才真正删除（防误触删掉几 GB 的模型）
  if (pendingDelete.value !== name) {
    pendingDelete.value = name;
    window.clearTimeout(pendingDeleteTimer);
    pendingDeleteTimer = window.setTimeout(() => (pendingDelete.value = ""), 3000);
    return;
  }
  pendingDelete.value = "";
  window.clearTimeout(pendingDeleteTimer);
  await invoke("cmd_model_delete", { name }).catch((e) => (status.value = String(e)));
  await refreshModelStatus();
  showToast("模型已删除");
}

async function changeWhisperModel(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  whisperModel.value = v;
  await invoke("cmd_set_setting", { key: "whisper_model", value: v }).catch((e) =>
    (status.value = String(e))
  );
  status.value = "识别档位已保存，下次播放生效";
}

async function wizardDownloadAll() {
  wizardDownloading.value = true;
  await download("whisper-small");
  await download("translation");
  wizardDownloading.value = false;
}

async function wizardDismiss() {
  await invoke("cmd_set_setting", { key: "wizard_done", value: "1" });
  showWizard.value = false;
}

async function changeEngine(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  engine.value = v;
  await invoke("cmd_mt_set_engine", { engine: v }).catch((e) => (status.value = String(e)));
}

async function downloadLocalModel() {
  if (downloading.value) return;
  await invoke("cmd_model_download", { name: "translation" }).catch((e) => (status.value = String(e)));
}

onUnmounted(() => {
  unlisten?.();
  window.removeEventListener("keydown", onKey);
});

// ---- 控制 ----
async function togglePause() {
  try {
    paused.value = await invoke<boolean>("cmd_player_toggle_pause");
  } catch (e) {
    status.value = String(e);
  }
}

async function toggleFullscreen() {
  try {
    await invoke("cmd_player_toggle_fullscreen");
  } catch (e) {
    status.value = String(e);
  }
}

async function seekAbsolute(t: number) {
  try {
    await invoke("cmd_player_seek", { target: t, absolute: true });
    timePos.value = t;
    displayTime.value = t;
  } catch (e) {
    status.value = String(e);
  }
}

async function seekBy(delta: number) {
  const next = Math.min(duration.value, Math.max(0, timePos.value + delta));
  await seekAbsolute(next);
  showToast(`${delta > 0 ? "快进" : "快退"} ${Math.abs(delta)}s → ${fmtTime(next)}`);
}

async function screenshot() {
  try {
    const p = await invoke<string>("cmd_player_screenshot");
    showToast(`截图已保存到桌面`);
    void p;
  } catch (e) {
    status.value = String(e);
  }
}

async function changeVolume(e: Event) {
  const v = Number((e.target as HTMLInputElement).value);
  volume.value = v;
  await invoke("cmd_player_set_volume", { volume: v }).catch((e) => (status.value = String(e)));
}

function fmtSpeed(s: number): string {
  return `${Math.round(s * 100) / 100}x`;
}

async function setSpeed(s: number) {
  // 0.1 步进、钳制 0.1~3，四舍五入消除浮点误差
  const next = Math.min(3, Math.max(0.1, Math.round(s * 10) / 10));
  speed.value = next;
  speedPopOpen.value = false;
  await invoke("cmd_player_set_speed", { speed: next }).catch((e) => (status.value = String(e)));
  showToast(`倍速 ${fmtSpeed(next)}`);
}

// ---- 滚轮：音量；Shift+滚轮：倍速微调（PotPlayer 习惯） ----
async function onWheel(e: WheelEvent) {
  if (!loaded.value) return;
  if (e.shiftKey) {
    await setSpeed(speed.value + (e.deltaY < 0 ? 0.1 : -0.1));
  } else {
    const v = Math.min(100, Math.max(0, volume.value + (e.deltaY < 0 ? 5 : -5)));
    await changeVolumeTo(v);
    showToast(`音量 ${Math.round(v)}`);
  }
}

// ---- 右键菜单（防溢出屏幕） ----
function openCtx(e: MouseEvent) {
  ctxX.value = Math.min(e.clientX, window.innerWidth - 230);
  ctxY.value = Math.min(e.clientY, window.innerHeight - 420);
  ctxOpen.value = true;
}

// ---- 播放中鼠标静止 2.5s 隐藏顶栏/控制栏，动一下即恢复 ----
function onUiActivity() {
  idle.value = false;
  window.clearTimeout(idleTimer);
  idleTimer = window.setTimeout(() => {
    if (
      loaded.value && !paused.value && !dragging.value &&
      !settingsOpen.value && !speedPopOpen.value && !ctxOpen.value
    ) {
      idle.value = true;
    }
  }, 2500);
}
const uiHidden = computed(
  () =>
    idle.value && loaded.value && !paused.value && !dragging.value &&
    !settingsOpen.value && !speedPopOpen.value && !ctxOpen.value
);

// ---- 进度条拖动 + 悬停预览 ----
function ratioAt(e: PointerEvent): number {
  const bar = barRef.value;
  if (!bar || !duration.value) return 0;
  const rect = bar.getBoundingClientRect();
  return Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
}
function startDrag(e: PointerEvent) {
  dragging.value = true;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
  displayTime.value = ratioAt(e) * duration.value;
}
function onBarMove(e: PointerEvent) {
  hoverTime.value = ratioAt(e) * duration.value;
  if (dragging.value) displayTime.value = hoverTime.value;
}
function onBarLeave() {
  hovering.value = false;
}
function endDrag(e: PointerEvent) {
  if (!dragging.value) return;
  displayTime.value = ratioAt(e) * duration.value;
  dragging.value = false;
  seekAbsolute(displayTime.value);
}

// ---- 打开文件 ----
async function openFile() {
  const path = await open({
    multiple: false,
    filters: [{ name: "视频", extensions: VIDEO_EXTS }],
  });
  if (typeof path === "string") await loadVideo(path);
}

async function onDrop(e: DragEvent) {
  const files = e.dataTransfer?.files;
  if (!files?.length) return;
  // Tauri 拖放：用 webviewWindow 事件拿真实路径（file:// 输入只兜底）
  const path = decodeURIComponent(files[0].path || "");
  if (path) await loadVideo(path);
}

async function loadVideo(path: string) {
  status.value = `加载中…`;
  try {
    await invoke("cmd_player_load", { path });
    filename.value = path.split("/").pop() || path;
    loaded.value = true;
  } catch (e) {
    status.value = String(e);
  }
}

// ---- 输入接管（技术方案决策 A：视频区点击全在 WebView） ----
function onStageClick() {
  if (speedPopOpen.value) {
    speedPopOpen.value = false;
    return;
  }
  if (loaded.value) togglePause();
}

function onKey(e: KeyboardEvent) {
  if (!loaded.value) return;
  switch (e.key) {
    case " ":
      e.preventDefault();
      togglePause();
      break;
    case "1":
      mode.value = "译文";
      break;
    case "2":
      mode.value = "原文";
      break;
    case "3":
      mode.value = "双语";
      break;
    case "ArrowLeft":
      seekBy(-5);
      break;
    case "ArrowRight":
      seekBy(5);
      break;
    case "ArrowUp":
      e.preventDefault();
      changeVolumeTo(Math.min(100, volume.value + 5));
      break;
    case "ArrowDown":
      e.preventDefault();
      changeVolumeTo(Math.max(0, volume.value - 5));
      break;
    case "f":
    case "F":
      toggleFullscreen();
      break;
    case "s":
    case "S":
      screenshot();
      break;
    case "[":
      setSpeed(speed.value - 0.1);
      break;
    case "]":
      setSpeed(speed.value + 0.1);
      break;
    case "Escape":
      ctxOpen.value = false;
      speedPopOpen.value = false;
      settingsOpen.value = false;
      break;
  }
}

async function changeVolumeTo(v: number) {
  volume.value = v;
  await invoke("cmd_player_set_volume", { volume: v }).catch((e) => (status.value = String(e)));
}

function fmtTime(t: number): string {
  if (!Number.isFinite(t)) return "0:00";
  const s = Math.floor(t % 60);
  const m = Math.floor((t / 60) % 60);
  const h = Math.floor(t / 3600);
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return h > 0 ? `${h}:${mm}:${String(s).padStart(2, "0")}` : `${mm}:${String(s).padStart(2, "0")}`;
}

// 注册 Tauri 原生拖放（拿真实文件路径）
onMounted(async () => {
  await getCurrentWebviewWindow().onDragDropEvent((ev) => {
    if (ev.payload.type === "drop" && ev.payload.paths.length > 0) {
      loadVideo(ev.payload.paths[0]);
    }
  });
});
</script>

<style scoped>
.shell {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  color: #f2f2f2;
  font-family: -apple-system, "PingFang SC", "Microsoft YaHei", sans-serif;
  user-select: none;
}

.topbar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
  background: rgba(16, 16, 20, 0.72);
  -webkit-app-region: drag;
}

.filename {
  flex: 1;
  font-size: 12px;
  color: #b9b9c4;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.bar-btn {
  -webkit-app-region: no-drag;
  padding: 4px 14px;
  border-radius: 6px;
  border: 1px solid rgba(255, 255, 255, 0.25);
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
  cursor: pointer;
  font-size: 12px;
}

/* 关键：视频区域背景透明，透出 mpv 子窗口 */
.stage {
  flex: 1;
  position: relative;
  background: transparent;
  cursor: default;
}

/* 窗口本身 transparent: true（字幕 overlay 依赖），未加载视频时 mpv 子窗口为空，
   整窗会看穿到桌面/其他应用——此时给不透明底色；加载后必须保持透明透出画面 */
.stage-empty {
  background: #101014;
}

.panel {
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 28px 40px;
  border-radius: 12px;
  background: rgba(15, 15, 22, 0.78);
  backdrop-filter: blur(8px);
  border: 1px solid rgba(255, 255, 255, 0.12);
  text-align: center;
}

.panel p {
  margin: 0;
  font-size: 13px;
  color: #cfcfd8;
}

.btn {
  padding: 8px 18px;
  border-radius: 6px;
  border: none;
  background: #4f6ef7;
  color: #fff;
  cursor: pointer;
  font-size: 13px;
}

.btn.ghost {
  background: rgba(255, 255, 255, 0.14);
}

.status {
  color: #ffd479;
  min-height: 16px;
}

/* 字幕 Overlay：底部居中，黑描边白字 */
.subtitle-overlay {
  position: absolute;
  left: 50%;
  bottom: 8%;
  transform: translateX(-50%);
  max-width: 80%;
  text-align: center;
  pointer-events: none;
}

.sub-src {
  font-size: 28px;
  font-weight: 700;
  color: #fff;
  text-shadow:
    0 0 4px #000, 0 0 4px #000,
    2px 2px 2px #000, -2px 2px 2px #000,
    2px -2px 2px #000, -2px -2px 2px #000;
  line-height: 1.4;
}

.sub-dst {
  margin-top: 6px;
  font-size: 24px;
  font-weight: 600;
  color: #ffd479;
  text-shadow:
    0 0 4px #000, 0 0 4px #000,
    2px 2px 2px #000, -2px 2px 2px #000,
    2px -2px 2px #000, -2px -2px 2px #000;
  line-height: 1.4;
}

.sub-dst.pending {
  font-size: 14px;
  font-weight: 400;
  color: rgba(255, 255, 255, 0.55);
}

/* 识别进度指示 */
.asr-progress {
  position: absolute;
  top: 14px;
  right: 16px;
  padding: 4px 12px;
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.5);
  color: rgba(255, 255, 255, 0.75);
  font-size: 11px;
  pointer-events: none;
  font-variant-numeric: tabular-nums;
}

.asr-error {
  position: absolute;
  top: 14px;
  left: 16px;
  padding: 6px 12px;
  border-radius: 8px;
  background: rgba(120, 30, 30, 0.8);
  color: #ffd7d7;
  font-size: 12px;
  max-width: 60%;
  pointer-events: none;
}

/* 设置抽屉 */
.drawer-mask {
  position: fixed;
  inset: 0;
  z-index: 30;
  background: rgba(0, 0, 0, 0.35);
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.drawer {
  position: fixed;
  top: 0;
  right: 0;
  bottom: 0;
  width: 320px;
  z-index: 31;
  padding: 16px 18px;
  background: rgba(22, 22, 28, 0.97);
  border-left: 1px solid rgba(255, 255, 255, 0.12);
  box-shadow: -8px 0 24px rgba(0, 0, 0, 0.45);
  transform: translateX(105%);
  transition: transform 0.22s ease;
  overflow-y: auto;
}

.drawer.open {
  transform: translateX(0);
}

.drawer-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 15px;
  font-weight: 700;
  margin-bottom: 10px;
}

.drawer-close {
  background: none;
  border: none;
  color: #9a9aa5;
  font-size: 14px;
  cursor: pointer;
  padding: 2px 6px;
}

.drawer-close:hover {
  color: #fff;
}

.drawer-section {
  margin: 14px 0 8px;
  font-size: 11px;
  font-weight: 700;
  color: #8f8f9a;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.set-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  font-size: 13px;
  margin: 0 0 10px;
}

.set-row select {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 6px;
  padding: 4px 8px;
  font-size: 12px;
}

.set-hint {
  margin: 8px 0 0;
  font-size: 11px;
  color: #8f8f9a;
  max-width: 320px;
}

.model-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
  font-size: 11px;
}

.model-desc {
  flex: 1;
  color: #b9b9c4;
}

.model-ok {
  color: #7ddb8a;
  white-space: nowrap;
}

/* 首启向导 */
.wizard-title {
  font-size: 18px;
  font-weight: 700;
  color: #fff;
}

.wizard-text {
  line-height: 1.8;
}

.wizard-actions {
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 100%;
}

/* 本地模型下载引导条 */
.cta-bar {
  position: absolute;
  top: 50px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
  border-radius: 10px;
  background: rgba(70, 50, 12, 0.9);
  border: 1px solid rgba(255, 212, 121, 0.4);
  font-size: 12px;
  color: #ffe1a1;
  max-width: 72%;
}

/* 新版本提示条 */
.update-bar {
  position: absolute;
  top: 50px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 16px;
  border-radius: 10px;
  background: rgba(30, 60, 110, 0.9);
  border: 1px solid rgba(121, 169, 255, 0.4);
  font-size: 12px;
  color: #cfe1ff;
  z-index: 15;
}

.btn.small {
  padding: 5px 12px;
  font-size: 12px;
  white-space: nowrap;
}

/* 控制栏 */
.controls {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 16px;
  background: rgba(16, 16, 20, 0.72);
}

.ctl-btn {
  border: none;
  background: transparent;
  color: #fff;
  font-size: 16px;
  cursor: pointer;
  padding: 4px 6px;
}

.time {
  font-size: 12px;
  color: #cfcfd8;
  font-variant-numeric: tabular-nums;
  min-width: 52px;
  text-align: center;
}

.progress {
  flex: 1;
  position: relative;
  height: 14px;
  display: flex;
  align-items: center;
  cursor: pointer;
}

.progress::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  height: 5px;
  border-radius: 3px;
  background: rgba(255, 255, 255, 0.2);
  transition: height 0.12s;
}

.progress:hover::before {
  height: 8px;
}

.progress-fill {
  position: absolute;
  left: 0;
  height: 5px;
  border-radius: 3px;
  background: #4f6ef7;
  transition: height 0.12s;
}

.progress:hover .progress-fill {
  height: 8px;
}

.progress-thumb {
  position: absolute;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: #fff;
  transform: translateX(-50%);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.5);
  opacity: 0;
  transition: opacity 0.12s;
}

.progress:hover .progress-thumb {
  opacity: 1;
}

.progress-tip {
  position: absolute;
  bottom: 18px;
  transform: translateX(-50%);
  padding: 3px 8px;
  border-radius: 4px;
  background: rgba(10, 10, 14, 0.92);
  color: #fff;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  pointer-events: none;
  white-space: nowrap;
}

.osd {
  position: absolute;
  left: 50%;
  top: 18px;
  transform: translateX(-50%);
  padding: 8px 16px;
  border-radius: 6px;
  background: rgba(10, 10, 14, 0.85);
  color: #fff;
  font-size: 13px;
  pointer-events: none;
  z-index: 10;
}

/* 倍速按钮 + 弹层 */
.speed-wrap {
  position: relative;
}

.speed-btn {
  min-width: 48px;
  padding: 4px 8px;
  border-radius: 6px;
  border: 1px solid rgba(255, 255, 255, 0.2);
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
  font-size: 12px;
  cursor: pointer;
}

.speed-pop {
  position: absolute;
  bottom: 36px;
  left: 50%;
  transform: translateX(-50%);
  padding: 10px;
  border-radius: 10px;
  background: rgba(24, 24, 30, 0.97);
  border: 1px solid rgba(255, 255, 255, 0.14);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  z-index: 35;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.speed-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px;
}

.speed-chip {
  padding: 5px 0;
  border: none;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.08);
  color: #e8e8ee;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.speed-chip:hover {
  background: rgba(255, 255, 255, 0.16);
}

.speed-chip.on {
  background: #4f6ef7;
  color: #fff;
}

/* 右键菜单 */
.ctx-mask {
  position: fixed;
  inset: 0;
  z-index: 40;
}

.ctx-menu {
  position: fixed;
  z-index: 41;
  min-width: 210px;
  padding: 6px;
  border-radius: 10px;
  background: rgba(24, 24, 30, 0.97);
  border: 1px solid rgba(255, 255, 255, 0.14);
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
  display: flex;
  flex-direction: column;
}

.ctx-item {
  text-align: left;
  padding: 7px 12px;
  border: none;
  background: none;
  color: #e8e8ee;
  font-size: 13px;
  border-radius: 6px;
  cursor: pointer;
}

.ctx-item:hover {
  background: rgba(255, 255, 255, 0.1);
}

.ctx-item.on {
  color: #ffd479;
}

.ctx-divider {
  height: 1px;
  background: rgba(255, 255, 255, 0.1);
  margin: 6px 4px;
}

.ctx-label {
  padding: 2px 12px 4px;
  font-size: 11px;
  color: #8f8f9a;
}

.ctx-speeds {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px;
  padding: 0 6px 6px;
}

.ctx-speed {
  padding: 5px 0;
  border: none;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.08);
  color: #e8e8ee;
  font-size: 12px;
  cursor: pointer;
}

.ctx-speed:hover {
  background: rgba(255, 255, 255, 0.16);
}

.ctx-speed.on {
  background: #4f6ef7;
  color: #fff;
}

/* 播放中鼠标静止：顶栏上滑、控制栏下滑、隐藏光标 */
.topbar,
.controls {
  transition: transform 0.25s ease;
}

.ui-hidden .topbar {
  transform: translateY(-100%);
}

.ui-hidden .controls {
  transform: translateY(100%);
}

.ui-hidden {
  cursor: none;
}

.vol-label {
  font-size: 12px;
  color: #9a9aa5;
}

.vol {
  width: 90px;
}

.vol-num {
  font-size: 12px;
  color: #cfcfd8;
  width: 26px;
  font-variant-numeric: tabular-nums;
}
</style>
