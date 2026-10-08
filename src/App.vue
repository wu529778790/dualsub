<template>
  <div class="shell">
    <header class="topbar">
      <span class="title">DualSub</span>
      <span class="subtitle">spike-1：透明 WebView + libmpv 子窗口</span>
      <button class="bar-btn" @click="togglePause">{{ paused ? "播放" : "暂停" }}</button>
    </header>

    <!-- 视频区域：背景全透明，透出下层 mpv 画面；点击计数证明 WebView 接管输入 -->
    <main class="stage" @click="clicks++">
      <div v-if="!loaded" class="panel">
        <p>1. 生成/填入一个本地视频路径</p>
        <p>2. 点「加载」——视频应出现在本面板<b>下方</b>，本面板文字浮在视频上</p>
        <input v-model="videoPath" class="path-input" spellcheck="false" />
        <button class="btn" @click.stop="load">加载</button>
        <button class="btn ghost" @click.stop="genTest">生成测试视频</button>
        <p class="status">{{ status }}</p>
      </div>
      <div v-else class="overlay-hint">
        点击计数：{{ clicks }}（证明点击落在 WebView 而非穿透到 mpv）
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

const videoPath = ref("");
const status = ref("");
const loaded = ref(false);
const paused = ref(false);
const clicks = ref(0);

async function load() {
  status.value = "加载中…";
  try {
    await invoke("cmd_mpv_load", { path: videoPath.value });
    loaded.value = true;
  } catch (e) {
    status.value = String(e);
  }
}

async function togglePause() {
  try {
    paused.value = await invoke<boolean>("cmd_mpv_toggle_pause");
  } catch (e) {
    status.value = String(e);
  }
}

async function genTest() {
  status.value = "正在生成测试视频…";
  try {
    videoPath.value = await invoke<string>("cmd_gen_test_video");
    status.value = "已生成，点「加载」";
  } catch (e) {
    status.value = String(e);
  }
}
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
  background: rgba(20, 20, 24, 0.55);
  -webkit-app-region: drag;
}

.title {
  font-size: 16px;
  font-weight: 700;
}

.subtitle {
  font-size: 12px;
  color: #b9b9c4;
  flex: 1;
}

.bar-btn {
  -webkit-app-region: no-drag;
  cursor: pointer;
}

/* 关键：视频区域背景透明，透出 mpv 子窗口 */
.stage {
  flex: 1;
  position: relative;
  background: transparent;
}

.panel {
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 28px 32px;
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

.path-input {
  width: 440px;
  padding: 8px 10px;
  border-radius: 6px;
  border: 1px solid rgba(255, 255, 255, 0.2);
  background: rgba(0, 0, 0, 0.4);
  color: #fff;
  font-size: 12px;
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
}

.overlay-hint {
  position: absolute;
  left: 50%;
  bottom: 24px;
  transform: translateX(-50%);
  padding: 8px 16px;
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.55);
  font-size: 12px;
  color: #eee;
}
</style>
