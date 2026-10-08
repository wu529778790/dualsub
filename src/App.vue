<template>
  <div class="shell">
    <header class="topbar">
      <span class="title">DualSub</span>
      <span class="filename">{{ filename }}</span>
      <button class="bar-btn" @click="openFile">打开</button>
    </header>

    <!-- 视频区域：背景全透明，透出下层 mpv 画面；输入由前端接管 -->
    <main
      class="stage"
      @click="onStageClick"
      @dblclick="toggleFullscreen"
      @dragover.prevent
      @drop.prevent="onDrop"
    >
      <div v-if="!loaded" class="panel">
        <p>把视频文件拖进窗口，或点「打开」</p>
        <button class="btn ghost" @click.stop="genTest">生成测试视频</button>
        <p class="status">{{ status }}</p>
      </div>
    </main>

    <!-- 控制栏 -->
    <footer v-if="loaded" class="controls">
      <button class="ctl-btn" @click="togglePause">{{ paused ? "▶" : "⏸" }}</button>
      <span class="time">{{ fmtTime(displayTime) }}</span>
      <div
        ref="barRef"
        class="progress"
        @pointerdown="startDrag"
        @pointermove="onDrag"
        @pointerup="endDrag"
        @pointercancel="endDrag"
      >
        <div class="progress-fill" :style="{ width: progressPct }" />
        <div class="progress-thumb" :style="{ left: progressPct }" />
      </div>
      <span class="time">/ {{ fmtTime(duration) }}</span>
      <select class="speed" :value="speed" @change="changeSpeed">
        <option :value="0.5">0.5x</option>
        <option :value="0.75">0.75x</option>
        <option :value="1">1x</option>
        <option :value="1.25">1.25x</option>
        <option :value="1.5">1.5x</option>
        <option :value="2">2x</option>
      </select>
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
      <button class="ctl-btn" @click="toggleFullscreen">⛶</button>
    </footer>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open } from "@tauri-apps/plugin-dialog";

const VIDEO_EXTS = ["mp4", "mkv", "avi", "mov", "webm", "m4v", "ts", "flv", "wmv", "rmvb"];

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

const barRef = ref<HTMLElement | null>(null);

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
});

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
}

async function changeVolume(e: Event) {
  const v = Number((e.target as HTMLInputElement).value);
  volume.value = v;
  await invoke("cmd_player_set_volume", { volume: v }).catch((e) => (status.value = String(e)));
}

async function changeSpeed(e: Event) {
  const s = Number((e.target as HTMLSelectElement).value);
  speed.value = s;
  await invoke("cmd_player_set_speed", { speed: s }).catch((e) => (status.value = String(e)));
}

// ---- 进度条拖动 ----
function startDrag(e: PointerEvent) {
  dragging.value = true;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
  updateDrag(e);
}
function onDrag(e: PointerEvent) {
  if (dragging.value) updateDrag(e);
}
function endDrag(e: PointerEvent) {
  if (!dragging.value) return;
  updateDrag(e);
  dragging.value = false;
  seekAbsolute(displayTime.value);
}
function updateDrag(e: PointerEvent) {
  const bar = barRef.value;
  if (!bar || !duration.value) return;
  const rect = bar.getBoundingClientRect();
  const ratio = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
  displayTime.value = ratio * duration.value;
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

async function genTest() {
  status.value = "正在生成测试视频…";
  try {
    const p = await invoke<string>("cmd_gen_test_video");
    await loadVideo(p);
  } catch (e) {
    status.value = String(e);
  }
}

// ---- 输入接管（技术方案决策 A：视频区点击全在 WebView） ----
function onStageClick() {
  if (loaded.value) togglePause();
}

function onKey(e: KeyboardEvent) {
  if (!loaded.value) return;
  switch (e.key) {
    case " ":
      e.preventDefault();
      togglePause();
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

.title {
  font-size: 15px;
  font-weight: 700;
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
  height: 4px;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.2);
}

.progress-fill {
  position: absolute;
  left: 0;
  height: 4px;
  border-radius: 2px;
  background: #4f6ef7;
}

.progress-thumb {
  position: absolute;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: #fff;
  transform: translateX(-50%);
}

.speed {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 6px;
  padding: 4px 6px;
  font-size: 12px;
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
