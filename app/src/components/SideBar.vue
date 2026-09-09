<script setup lang="ts">
import {
  Sparkles,
  FileSearch,
  Copy,
  ChartPie,
  Package,
  Settings,
} from "lucide-vue-next"
import { store, type PageId } from "../store"

const items: { id: PageId; label: string; icon: any }[] = [
  { id: "clean", label: "一键清理", icon: Sparkles },
  { id: "large", label: "大文件", icon: FileSearch },
  { id: "dupes", label: "重复文件", icon: Copy },
  { id: "space", label: "空间分析", icon: ChartPie },
  { id: "uninstall", label: "应用卸载", icon: Package },
  { id: "settings", label: "设置", icon: Settings },
]
</script>

<template>
  <aside class="sidebar">
    <div class="logo-row">
      <img src="../assets/logo.png" class="logo" alt="DiskClear" />
      <div class="logo-text">
        <div class="name">DiskClear</div>
        <div class="sub">盘清</div>
      </div>
    </div>

    <nav class="nav">
      <button
        v-for="item in items"
        :key="item.id"
        class="nav-item"
        :class="{ active: store.page === item.id }"
        @click="store.go(item.id)"
      >
        <component :is="item.icon" :size="18" stroke-width="1.8" />
        <span>{{ item.label }}</span>
      </button>
    </nav>

    <div class="sidebar-footer">
      <div class="safety-tip">
        <span class="dot"></span>
        所有删除均可从恢复区撤销
      </div>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: 216px;
  flex-shrink: 0;
  background: var(--card);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  padding: 18px 12px;
}

.logo-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 10px 20px;
}

.logo {
  width: 38px;
  height: 38px;
  border-radius: 10px;
}

.logo-text .name {
  font-size: 17px;
  font-weight: 700;
  letter-spacing: 0.2px;
}

.logo-text .sub {
  font-size: 11px;
  color: var(--muted);
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 11px;
  border: none;
  background: transparent;
  color: var(--text-2);
  font-size: 14px;
  padding: 11px 12px;
  border-radius: 10px;
  cursor: pointer;
  text-align: left;
  transition: all 0.15s;
}

.nav-item:hover {
  background: var(--bg);
  color: var(--text);
}

.nav-item.active {
  background: var(--primary-soft);
  color: var(--primary);
  font-weight: 600;
}

.sidebar-footer {
  margin-top: auto;
  padding: 10px;
}

.safety-tip {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 11px;
  color: var(--accent);
  background: var(--accent-soft);
  padding: 9px 11px;
  border-radius: 9px;
}

.dot {
  width: 7px;
  height: 7px;
  border-radius: 99px;
  background: var(--accent);
  flex-shrink: 0;
}
</style>
