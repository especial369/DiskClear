<script setup lang="ts">
import { computed } from "vue"
import { ArchiveRestore } from "lucide-vue-next"
import { store } from "../store"
import { fmtSize } from "../types"

const freed = computed(() => fmtSize(store.stats.freedTotalBytes))
const recovery = computed(() => fmtSize(store.stats.recoveryBytes))
</script>

<template>
  <footer class="status-bar">
    <div class="left">
      <span class="stat">
        累计已释放 <b class="green">{{ freed }}</b>
      </span>
      <span class="divider">|</span>
      <button class="recovery-link" title="打开恢复区" @click="store.go('recovery')">
        <ArchiveRestore :size="13" stroke-width="2" />
        恢复区 <b>{{ recovery }}</b>
      </button>
    </div>
    <div class="right text-muted">DiskClear 盘清 · 本地扫描，不上传任何数据</div>
  </footer>
</template>

<style scoped>
.status-bar {
  height: 40px;
  flex-shrink: 0;
  border-top: 1px solid var(--border);
  background: var(--card);
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 18px;
  font-size: 12px;
  color: var(--text-2);
}

.left {
  display: flex;
  align-items: center;
  gap: 12px;
}

.green {
  color: var(--accent);
}

.divider {
  color: var(--border);
}

.recovery-link {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: none;
  background: transparent;
  color: var(--text-2);
  font-size: 12px;
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 7px;
}

.recovery-link:hover {
  background: var(--primary-soft);
  color: var(--primary);
}
</style>
