<script setup lang="ts">
import { onMounted, markRaw, type Component } from "vue"
import SideBar from "./components/SideBar.vue"
import StatusBar from "./components/StatusBar.vue"
import QuickClean from "./pages/QuickClean.vue"
import Recovery from "./pages/Recovery.vue"
import SettingsPage from "./pages/SettingsPage.vue"
import LargeFiles from "./pages/LargeFiles.vue"
import Duplicates from "./pages/Duplicates.vue"
import SpaceAnalysis from "./pages/SpaceAnalysis.vue"
import AppUninstall from "./pages/AppUninstall.vue"
import { store, applyTheme, type PageId } from "./store"
import { api } from "./api"

// keep-alive 缓存全部页面：切走不销毁、切回秒开，重负载数据（应用卸载列表 / 空间分析结果）随实例保留。
// markRaw 避免组件对象被响应式化（性能与告警）。
const pages: Record<PageId, Component> = {
  clean: markRaw(QuickClean),
  large: markRaw(LargeFiles),
  dupes: markRaw(Duplicates),
  recovery: markRaw(Recovery),
  settings: markRaw(SettingsPage),
  space: markRaw(SpaceAnalysis),
  uninstall: markRaw(AppUninstall),
}

onMounted(async () => {
  try {
    const s = await api.getSettings()
    applyTheme(s.theme)
  } catch {
    applyTheme("system")
  }
  store.refreshStats()
})
</script>

<template>
  <div class="app-shell">
    <SideBar />
    <div class="main-col">
      <main class="main-area">
        <keep-alive :max="10">
          <component :is="pages[store.page]" />
        </keep-alive>
      </main>
      <StatusBar />
    </div>

    <!-- 轻提示 -->
    <div class="toast-wrap">
      <div v-for="t in store.toasts" :key="t.id" class="toast" :class="t.kind">
        {{ t.text }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  height: 100vh;
}

.main-col {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.main-area {
  flex: 1;
  overflow-y: auto;
  padding: 22px 26px;
}

.toast-wrap {
  position: fixed;
  top: 18px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 100;
  display: flex;
  flex-direction: column;
  gap: 8px;
  pointer-events: none;
}

.toast {
  padding: 9px 18px;
  border-radius: 10px;
  font-size: 13px;
  color: #fff;
  box-shadow: var(--shadow);
  animation: slide-in 0.2s ease;
}

.toast.info {
  background: #334155;
}

.toast.success {
  background: var(--accent);
}

.toast.error {
  background: var(--danger);
}

@keyframes slide-in {
  from {
    opacity: 0;
    transform: translateY(-8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
</style>
