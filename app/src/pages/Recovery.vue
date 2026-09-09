<script setup lang="ts">
import { computed, onMounted, ref } from "vue"
import { ArchiveRestore, ChevronDown, FileQuestion, RotateCcw, Trash2 } from "lucide-vue-next"
import AppModal from "../components/AppModal.vue"
import { api } from "../api"
import { store } from "../store"
import { daysLeft, fmtSize, fmtTime, type RecoveryEntry } from "../types"

interface Batch {
  id: string
  entries: RecoveryEntry[]
  size: number
  time: number
  drives: string[]
}

const entries = ref<RecoveryEntry[]>([])
const expanded = ref<Record<string, boolean>>({})
const confirmClear = ref(false)
const confirmDelete = ref<Batch | null>(null)
const loading = ref(false)

const batches = computed<Batch[]>(() => {
  const map = new Map<string, Batch>()
  for (const e of entries.value) {
    let b = map.get(e.batchId)
    if (!b) {
      b = { id: e.batchId, entries: [], size: 0, time: e.deletedAtMs, drives: [] }
      map.set(e.batchId, b)
    }
    b.entries.push(e)
    b.size += e.size
    if (!b.drives.includes(e.drive)) b.drives.push(e.drive)
  }
  return [...map.values()].sort((a, b) => b.time - a.time)
})

const totalSize = computed(() => entries.value.reduce((s, e) => s + e.size, 0))

const catName: Record<string, string> = {
  temp: "临时文件",
  recycle: "回收站",
  browser: "浏览器缓存",
  update: "更新缓存",
  logs: "系统日志",
  thumbnail: "缩略图缓存",
  large: "大文件",
  dupes: "重复文件",
  unknown: "未知来源",
}

async function refresh() {
  loading.value = true
  try {
    entries.value = await api.listRecovery()
    store.refreshStats()
  } finally {
    loading.value = false
  }
}

onMounted(refresh)

async function restoreBatch(b: Batch) {
  const n = await api.restoreEntries(b.entries.map((e) => e.id))
  store.toast(`已恢复 ${n} 个文件到原位置`, "success")
  await refresh()
}

async function restoreOne(e: RecoveryEntry) {
  const n = await api.restoreEntries([e.id])
  store.toast(n > 0 ? "已恢复到原位置" : "恢复失败：文件可能已被清理", n > 0 ? "success" : "error")
  await refresh()
}

async function deleteBatch(b: Batch) {
  confirmDelete.value = null
  const n = await api.deleteRecovery(b.entries.map((e) => e.id))
  store.toast(`已永久删除 ${n} 个文件`, "success")
  await refresh()
}

async function clearAll() {
  confirmClear.value = false
  const n = await api.clearRecovery()
  store.toast(`已清空恢复区（${n} 个文件）`, "success")
  await refresh()
}

function toggle(bid: string) {
  expanded.value[bid] = !expanded.value[bid]
}
</script>

<template>
  <div class="recovery-page">
    <div class="head card">
      <div>
        <div class="head-title">恢复区</div>
        <div class="text-muted">
          被清理的文件在此保留 7 天（可在设置中调整），期间可随时恢复到原位置。
        </div>
      </div>
      <div class="head-right">
        <div class="total-size">{{ fmtSize(totalSize) }}</div>
        <div class="text-muted">{{ entries.length }} 个文件 · {{ batches.length }} 个批次</div>
        <button v-if="entries.length" class="btn btn-danger" @click="confirmClear = true">
          <Trash2 :size="14" stroke-width="2" /> 清空全部
        </button>
      </div>
    </div>

    <div v-if="!batches.length && !loading" class="empty card">
      <FileQuestion :size="40" stroke-width="1.4" />
      <div class="empty-title">恢复区是空的</div>
      <div class="text-muted">执行一次清理后，被删除的文件会出现在这里。</div>
      <button class="btn btn-primary" @click="store.go('clean')">去清理</button>
    </div>

    <div v-for="b in batches" :key="b.id" class="batch card">
      <div class="batch-head" @click="toggle(b.id)">
        <ChevronDown :size="16" class="chev" :class="{ open: expanded[b.id] }" stroke-width="2" />
        <div class="batch-info">
          <div class="batch-title">
            {{ fmtTime(b.time) }} · {{ catName[b.entries[0].categoryId] || "清理" }}
            <span class="drive-tag">{{ b.drives.join("/") }} 盘</span>
          </div>
          <div class="text-muted">{{ b.entries.length }} 个文件 · {{ fmtSize(b.size) }}</div>
        </div>
        <div class="batch-actions" @click.stop>
          <button class="btn" @click="restoreBatch(b)">
            <RotateCcw :size="14" stroke-width="2" /> 全部恢复
          </button>
          <button class="btn btn-danger" @click="confirmDelete = b">
            <Trash2 :size="14" stroke-width="2" /> 删除
          </button>
        </div>
      </div>

      <div v-show="expanded[b.id]" class="file-list">
        <div v-for="e in b.entries" :key="e.id" class="file-row">
          <div class="file-info">
            <div class="file-name" :title="e.originalPath">{{ e.originalPath }}</div>
            <div class="text-muted">{{ fmtSize(e.size) }} · 剩余保留 {{ daysLeft(e.expiresAtMs) }} 天</div>
          </div>
          <button class="btn btn-sm" @click="restoreOne(e)">
            <ArchiveRestore :size="13" stroke-width="2" /> 恢复
          </button>
        </div>
      </div>
    </div>

    <!-- 清空确认 -->
    <AppModal :open="confirmClear" title="清空恢复区？" danger @close="confirmClear = false">
      恢复区中的 <b>{{ entries.length }}</b> 个文件（{{ fmtSize(totalSize) }}）将被<b>永久删除，无法恢复</b>。确定继续？
      <template #footer>
        <button class="btn" @click="confirmClear = false">取消</button>
        <button class="btn btn-primary" style="background: var(--danger); border-color: var(--danger)" @click="clearAll">
          永久删除
        </button>
      </template>
    </AppModal>

    <!-- 删除批次确认 -->
    <AppModal :open="!!confirmDelete" title="永久删除该批次？" danger @close="confirmDelete = null">
      <template v-if="confirmDelete">
        该批次 <b>{{ confirmDelete.entries.length }}</b> 个文件（{{ fmtSize(confirmDelete.size) }}）将被<b>永久删除，无法恢复</b>。确定继续？
      </template>
      <template #footer>
        <button class="btn" @click="confirmDelete = null">取消</button>
        <button class="btn btn-primary" style="background: var(--danger); border-color: var(--danger)" @click="confirmDelete && deleteBatch(confirmDelete)">
          永久删除
        </button>
      </template>
    </AppModal>
  </div>
</template>

<style scoped>
.recovery-page {
  max-width: 980px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 20px 24px;
}

.head-title {
  font-size: 18px;
  font-weight: 700;
  margin-bottom: 4px;
}

.head-right {
  display: flex;
  align-items: center;
  gap: 14px;
}

.total-size {
  font-size: 24px;
  font-weight: 800;
  color: var(--primary);
}

.empty {
  padding: 52px;
  text-align: center;
  color: var(--muted);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.empty-title {
  font-size: 16px;
  font-weight: 700;
  color: var(--text);
}

.batch {
  overflow: hidden;
}

.batch-head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px 18px;
  cursor: pointer;
}

.batch-head:hover {
  background: var(--bg);
}

.chev {
  transition: transform 0.15s;
  color: var(--muted);
}

.chev.open {
  transform: rotate(180deg);
}

.batch-info {
  flex: 1;
}

.batch-title {
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 3px;
}

.drive-tag {
  font-size: 11px;
  font-weight: 600;
  color: var(--primary);
  background: var(--primary-soft);
  border-radius: 99px;
  padding: 2px 9px;
  margin-left: 6px;
}

.batch-actions {
  display: flex;
  gap: 8px;
}

.file-list {
  border-top: 1px solid var(--border);
  padding: 6px 18px 12px 46px;
  max-height: 420px;
  overflow-y: auto;
}

.file-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 9px 0;
  border-bottom: 1px dashed var(--border);
}

.file-row:last-child {
  border-bottom: none;
}

.file-info {
  flex: 1;
  min-width: 0;
}

.file-name {
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.btn-sm {
  padding: 5px 10px;
  font-size: 12px;
}
</style>
