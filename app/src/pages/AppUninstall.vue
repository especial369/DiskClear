<script setup lang="ts">
// 应用卸载（PRD 2.6 v1 简单版）：列表（注册表 Uninstall 键 + UWP）→ 调起自带卸载器
// → 目录级残留提示（启发式匹配，用户勾选确认）→ 移入恢复区。
// 注册表残留清理明确不在 v1 范围（PRD 2.6 排除项）。
import { computed, onMounted, onUnmounted, reactive, ref } from "vue"
import { listen } from "@tauri-apps/api/event"
import { PackageOpen, RefreshCw, Search, Trash2 } from "lucide-vue-next"
import AppModal from "../components/AppModal.vue"
import { api } from "../api"
import { store } from "../store"
import {
  fmtSize,
  type CleanProgress,
  type CleanResult,
  type InstalledApp,
  type ResidueDir,
} from "../types"

const apps = ref<InstalledApp[]>([])
const loading = ref(false)
const keyword = ref("")

// 卸载流程
const confirmOpen = ref(false)
const target = ref<InstalledApp | null>(null)
const waitingExit = ref(false)
const pending = ref<InstalledApp | null>(null)

// 残留流程
const residues = ref<ResidueDir[]>([])
const residueApp = ref<InstalledApp | null>(null)
const residueSelected = reactive<Record<string, boolean>>({})
const scanningResidue = ref(false)
const confirmResidueOpen = ref(false)

// 清理流程
const cleaning = ref(false)
const cleanProgress = ref<CleanProgress | null>(null)
const resultOpen = ref(false)
const cleanResult = ref<CleanResult | null>(null)

let unlisteners: (() => void)[] = []

const filtered = computed(() => {
  const k = keyword.value.trim().toLowerCase()
  if (!k) return apps.value
  return apps.value.filter(
    (a) => a.name.toLowerCase().includes(k) || a.publisher.toLowerCase().includes(k),
  )
})

const residueSelectedList = computed(() => residues.value.filter((r) => residueSelected[r.path]))
const residueSelectedSize = computed(() =>
  residueSelectedList.value.reduce((s, r) => s + r.size, 0),
)

async function refresh() {
  loading.value = true
  try {
    apps.value = await api.listInstalledApps()
  } catch (e) {
    store.toast(String(e), "error")
  } finally {
    loading.value = false
  }
}

function clearResidueSelection() {
  for (const k of Object.keys(residueSelected)) delete residueSelected[k]
}

onMounted(async () => {
  refresh()
  unlisteners.push(
    await listen<{ displayName: string; ok: boolean }>("uninstall-exited", async () => {
      waitingExit.value = false
      const app = pending.value
      pending.value = null
      if (app) await scanResidue(app)
    }),
  )
})

onUnmounted(() => unlisteners.forEach((f) => f()))

function askUninstall(app: InstalledApp) {
  target.value = app
  confirmOpen.value = true
}

async function doUninstall() {
  const app = target.value
  if (!app) return
  confirmOpen.value = false
  waitingExit.value = true
  pending.value = app
  try {
    await api.launchUninstall(app)
    store.toast(`已调起「${app.name}」的卸载程序，完成后将自动扫描残留`, "info")
  } catch (e) {
    waitingExit.value = false
    pending.value = null
    store.toast(String(e), "error")
  }
}

async function scanResidue(app: InstalledApp) {
  scanningResidue.value = true
  try {
    const list = await api.scanResidues(app.name, app.publisher, app.installLocation)
    residues.value = list
    residueApp.value = app
    clearResidueSelection()
    for (const r of list) residueSelected[r.path] = true
    if (list.length === 0) {
      store.toast(`未发现「${app.name}」的目录级残留`, "success")
    }
    refresh() // 卸载完成后刷新程序列表
  } catch (e) {
    store.toast(String(e), "error")
  } finally {
    scanningResidue.value = false
  }
}

async function rescanResidue() {
  if (residueApp.value) await scanResidue(residueApp.value)
}

function skipResidue() {
  residues.value = []
  residueApp.value = null
  clearResidueSelection()
}

async function doCleanResidues() {
  confirmResidueOpen.value = false
  cleaning.value = true
  const un = await listen<CleanProgress>("clean-progress", (e) => {
    cleanProgress.value = e.payload
  })
  try {
    const dirs = residueSelectedList.value.map((r) => ({ path: r.path, size: r.size }))
    cleanResult.value = await api.cleanResidues(dirs)
    resultOpen.value = true
    store.refreshStats()
    skipResidue()
  } catch (e) {
    store.toast(String(e), "error")
  } finally {
    un()
    cleaning.value = false
  }
}

function sizeText(bytes: number): string {
  return bytes > 0 ? fmtSize(bytes) : "—"
}

function dateText(d: string): string {
  return d || "—"
}
</script>

<template>
  <div class="uninstall-page">
    <!-- 顶部：说明 + 搜索 + 刷新 -->
    <div class="head-card card">
      <div class="head-text">
        <div class="head-title">应用卸载</div>
        <div class="head-desc text-muted">
          共 {{ apps.length }} 个程序 · 调起程序自带卸载器 · 卸载后提示目录级残留（移入恢复区可撤销）；
          注册表清理规划于 v2
        </div>
      </div>
      <div class="head-tools">
        <div class="search-box">
          <Search :size="15" stroke-width="2" />
          <input v-model="keyword" placeholder="搜索程序名 / 发布者" />
        </div>
        <button class="btn" :disabled="loading" @click="refresh">
          <RefreshCw :size="15" stroke-width="2" :class="{ spinning: loading }" />
          刷新
        </button>
      </div>
    </div>

    <!-- 程序列表 -->
    <div class="app-list">
      <div v-if="loading && apps.length === 0" class="loading-hint text-muted">正在读取已安装程序…</div>
      <div v-else-if="filtered.length === 0" class="loading-hint text-muted">没有匹配的程序</div>
      <div v-for="a in filtered" :key="a.id" class="app-item card">
        <div class="app-main">
          <div class="app-title-row">
            <span class="app-name">{{ a.name }}</span>
            <span v-if="a.version" class="app-ver text-muted">{{ a.version }}</span>
            <span class="src-badge" :class="a.isUwp ? 'uwp' : 'win32'">
              {{ a.isUwp ? "UWP" : "应用" }}
            </span>
          </div>
          <div class="app-sub text-muted">
            {{ a.publisher || "未知发布者" }} · 安装于 {{ dateText(a.installDate) }}
          </div>
        </div>
        <div class="app-side">
          <div class="app-size">{{ sizeText(a.estimatedSize) }}</div>
          <button class="btn btn-sm" @click="askUninstall(a)">
            <PackageOpen :size="14" stroke-width="2" />
            卸载
          </button>
        </div>
      </div>
    </div>

    <!-- 残留区 -->
    <div v-if="residues.length && residueApp" class="residue-card card">
      <div class="residue-head">
        <div class="residue-title">
          <Trash2 :size="16" stroke-width="2" />
          「{{ residueApp.name }}」目录级残留（{{ residues.length }} 项）
        </div>
        <div class="residue-actions">
          <button class="btn btn-sm" :disabled="scanningResidue" @click="rescanResidue">重新扫描</button>
          <button class="btn btn-sm" @click="skipResidue">跳过</button>
        </div>
      </div>
      <div class="residue-desc text-muted">
        按程序名/发布者启发式匹配，仅列 %ProgramFiles% / %ProgramData% / %LOCALAPPDATA% 下的目录；
        勾选确认后移入恢复区，不会直接删除。
      </div>
      <label v-for="r in residues" :key="r.path" class="residue-item" :class="{ checked: residueSelected[r.path] }">
        <input v-model="residueSelected[r.path]" type="checkbox" class="residue-check" />
        <div class="residue-main">
          <div class="residue-path">{{ r.path }}</div>
          <div class="residue-reason text-muted">{{ r.reason }}</div>
        </div>
        <div class="residue-size">{{ fmtSize(r.size) }}</div>
      </label>
      <div class="residue-bar">
        <div class="text-muted">
          已选 <b>{{ residueSelectedList.length }}</b> 项 · 预计释放
          <b class="green">{{ fmtSize(residueSelectedSize) }}</b>
        </div>
        <button
          class="btn btn-primary"
          :disabled="residueSelectedList.length === 0 || cleaning"
          @click="confirmResidueOpen = true"
        >
          清理残留
        </button>
      </div>
    </div>

    <!-- 卸载确认 -->
    <AppModal :open="confirmOpen" title="确认卸载？" @close="confirmOpen = false">
      <p>
        即将调起 <b>{{ target?.name }}</b>{{ target?.version ? `（${target?.version}）` : "" }}
        的官方卸载程序。
      </p>
      <p class="text-muted">
        请在弹出的卸载向导中完成操作；卸载程序退出后，DiskClear 将自动扫描目录级残留。
      </p>
      <template #footer>
        <button class="btn" @click="confirmOpen = false">取消</button>
        <button class="btn btn-primary" @click="doUninstall">调起卸载器</button>
      </template>
    </AppModal>

    <!-- 等待卸载完成 -->
    <AppModal :open="waitingExit" title="正在等待卸载完成…" :closable="false">
      <div class="waiting-row">
        <div class="spinner"></div>
        <div>
          <div class="progress-text">请完成「{{ pending?.name }}」的卸载向导</div>
          <div class="text-muted">卸载程序退出后，将自动扫描目录级残留。</div>
        </div>
      </div>
      <template #footer><span></span></template>
    </AppModal>

    <!-- 残留清理确认 -->
    <AppModal :open="confirmResidueOpen" title="确认清理残留？" @close="confirmResidueOpen = false">
      <p>
        即将把 <b>{{ residueSelectedList.length }}</b> 个残留目录（共
        <b>{{ fmtSize(residueSelectedSize) }}</b>）移入恢复区，保留 7 天，可随时撤销。
      </p>
      <template #footer>
        <button class="btn" @click="confirmResidueOpen = false">取消</button>
        <button class="btn btn-primary" @click="doCleanResidues">确认清理</button>
      </template>
    </AppModal>

    <!-- 清理进度 -->
    <AppModal :open="cleaning" title="正在清理残留…" :closable="false">
      <div class="clean-progress">
        <div class="progress-track">
          <div
            class="progress-fill"
            :style="{
              width: cleanProgress ? (cleanProgress.done / Math.max(1, cleanProgress.total)) * 100 + '%' : '0%',
            }"
          ></div>
        </div>
        <div class="progress-meta text-muted">
          {{ cleanProgress?.done ?? 0 }} / {{ cleanProgress?.total ?? 0 }} 项 · 已释放
          {{ fmtSize(cleanProgress?.freedBytes ?? 0) }}
          <template v-if="cleanProgress?.skippedLocked">
            · 已跳过占用 {{ cleanProgress?.skippedLocked }} 项
          </template>
        </div>
      </div>
      <template #footer><span></span></template>
    </AppModal>

    <!-- 清理结果 -->
    <AppModal :open="resultOpen" title="残留清理完成 🎉" @close="resultOpen = false">
      <div v-if="cleanResult" class="result-grid">
        <div class="result-item">
          <div class="num green">{{ fmtSize(cleanResult.freedBytes) }}</div>
          <div class="text-muted">已释放空间</div>
        </div>
        <div class="result-item">
          <div class="num">{{ cleanResult.movedCount }}</div>
          <div class="text-muted">移入恢复区目录</div>
        </div>
        <div class="result-item">
          <div class="num">{{ cleanResult.skippedLocked }}</div>
          <div class="text-muted">因占用跳过</div>
        </div>
      </div>
      <p class="text-muted">残留目录保存在恢复区（7 天后自动清除），可在底部状态栏「恢复区」中查看或恢复。</p>
      <template #footer>
        <button class="btn" @click="resultOpen = false">关闭</button>
        <button class="btn btn-primary" @click="resultOpen = false; store.go('recovery')">查看恢复区</button>
      </template>
    </AppModal>
  </div>
</template>

<style scoped>
.uninstall-page {
  max-width: 980px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.head-card {
  padding: 18px 22px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  flex-wrap: wrap;
}

.head-title {
  font-size: 18px;
  font-weight: 700;
}

.head-desc {
  margin-top: 4px;
  font-size: 12px;
}

.head-tools {
  display: flex;
  align-items: center;
  gap: 10px;
}

.search-box {
  display: flex;
  align-items: center;
  gap: 8px;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 8px 12px;
  color: var(--muted);
}

.search-box input {
  border: none;
  background: transparent;
  outline: none;
  color: var(--text);
  font-size: 13px;
  width: 180px;
}

.btn-sm {
  padding: 7px 13px;
  font-size: 12px;
}

.spinning {
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.app-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.loading-hint {
  padding: 40px;
  text-align: center;
}

.app-item {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 13px 18px;
  border: 1.5px solid var(--border);
}

.app-main {
  flex: 1;
  min-width: 0;
}

.app-title-row {
  display: flex;
  align-items: center;
  gap: 9px;
}

.app-name {
  font-size: 14px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.app-ver {
  font-size: 12px;
}

.src-badge {
  font-size: 11px;
  padding: 1px 8px;
  border-radius: 99px;
  flex-shrink: 0;
}

.src-badge.win32 {
  color: var(--primary);
  background: var(--primary-soft);
}

.src-badge.uwp {
  color: var(--accent);
  background: var(--accent-soft);
}

.app-sub {
  margin-top: 3px;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.app-side {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-shrink: 0;
}

.app-size {
  font-size: 13px;
  font-weight: 600;
  color: var(--primary);
  min-width: 64px;
  text-align: right;
}

.residue-card {
  padding: 18px 20px;
  border: 1.5px solid var(--warn);
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.residue-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.residue-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 700;
  color: var(--warn);
}

.residue-actions {
  display: flex;
  gap: 8px;
}

.residue-desc {
  font-size: 12px;
}

.residue-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border: 1.5px solid var(--border);
  border-radius: 10px;
  cursor: pointer;
  transition: border-color 0.15s;
}

.residue-item.checked {
  border-color: var(--warn);
}

.residue-check {
  width: 16px;
  height: 16px;
  accent-color: var(--warn);
  flex-shrink: 0;
}

.residue-main {
  flex: 1;
  min-width: 0;
}

.residue-path {
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.residue-reason {
  font-size: 11px;
  margin-top: 2px;
}

.residue-size {
  font-size: 13px;
  font-weight: 600;
  flex-shrink: 0;
}

.residue-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-top: 6px;
}

.green {
  color: var(--accent);
}

.waiting-row {
  display: flex;
  align-items: center;
  gap: 16px;
}

.spinner {
  width: 22px;
  height: 22px;
  border: 3px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 99px;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}

.progress-text {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 4px;
}

.progress-track {
  height: 8px;
  background: var(--border);
  border-radius: 99px;
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: var(--primary);
  border-radius: 99px;
  transition: width 0.2s;
}

.progress-fill.indeterminate {
  width: 40%;
  animation: indet 1.1s ease-in-out infinite;
}

@keyframes indet {
  0% {
    margin-left: -40%;
  }
  100% {
    margin-left: 100%;
  }
}

.progress-meta {
  margin-top: 8px;
}

.result-grid {
  display: flex;
  gap: 12px;
  margin: 6px 0 10px;
}

.result-item {
  flex: 1;
  background: var(--bg);
  border-radius: 10px;
  padding: 14px;
  text-align: center;
}

.result-item .num {
  font-size: 20px;
  font-weight: 800;
}

.result-item .green {
  color: var(--accent);
}
</style>
