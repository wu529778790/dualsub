<template>
  <div class="shell">
    <header class="topbar">
      <span class="title">DualSub</span>
      <span class="subtitle">打开视频，字幕跟上</span>
    </header>
    <main class="stage">
      <div class="placeholder">
        <p>M0 脚手架已就绪</p>
        <p class="hint">下一步：M1 播放内核（libmpv 子窗口嵌入）</p>
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
// M0：空壳。M1 起在这里接入播放器状态与字幕 Overlay。
</script>

<style scoped>
/* 仅中文，不做 i18n。深色基调，为透明 WebView + 视频子窗口做准备 */
.shell {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  color: #e8e8e8;
  font-family: -apple-system, "PingFang SC", "Microsoft YaHei", sans-serif;
  user-select: none;
}

.topbar {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 10px 16px;
  background: rgba(20, 20, 24, 0.85);
  -webkit-app-region: drag;
}

.title {
  font-size: 16px;
  font-weight: 700;
}

.subtitle {
  font-size: 12px;
  color: #9a9aa5;
}

.stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(12, 12, 16, 0.92);
}

.placeholder {
  text-align: center;
  color: #6f6f7a;
}

.placeholder p {
  margin: 4px 0;
  font-size: 14px;
}

.hint {
  font-size: 12px;
}
</style>
