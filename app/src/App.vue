<script setup lang="ts">
import { onMounted } from "vue"
import SideBar from "./components/SideBar.vue"
import StatusBar from "./components/StatusBar.vue"
import QuickClean from "./pages/QuickClean.vue"
import Recovery from "./pages/Recovery.vue"
import SettingsPage from "./pages/SettingsPage.vue"
import LargeFiles from "./pages/LargeFiles.vue"
import Duplicates from "./pages/Duplicates.vue"
import Placeholder from "./components/Placeholder.vue"
import { store, applyTheme } from "./store"
import { api } from "./api"

const placeholders: Record<string, { milestone: string; title: string; desc: string; icon: string }> = {
  space: {
    milestone: "M3 里程碑交付",
    title: "磁盘空间分析",
    desc: "矩形树图（Treemap）可视化目录占用，点击下钻、悬停查看详情、右键快捷操作，看得见空间去哪了。",
    icon: "📊",
  },
  uninstall: {
    milestone: "M3 里程碑交付",
    title: "应用卸载",
    desc: "列出已安装程序，调起自带卸载器并提示目录级残留；注册表清理规划于 v2。",
    icon: "📦",
  },
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
        <QuickClean v-if="store.page === 'clean'" />
        <LargeFiles v-else-if="store.page === 'large'" />
        <Duplicates v-else-if="store.page === 'dupes'" />
        <Recovery v-else-if="store.page === 'recovery'" />
        <SettingsPage v-else-if="store.page === 'settings'" />
        <Placeholder
          v-else
          :milestone="placeholders[store.page].milestone"
          :title="placeholders[store.page].title"
          :desc="placeholders[store.page].desc"
          :icon="placeholders[store.page].icon"
        />
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
