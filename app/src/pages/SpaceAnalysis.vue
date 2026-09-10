<script setup lang="ts">
// 空间分析（PRD 2.5）：Treemap 可视化 + 目录下钻 + 右键快捷操作
import { onMounted, onUnmounted, ref } from "vue"
import { listen } from "@tauri-apps/api/event"
import { FolderOpen, ScanSearch, Trash2 } from "lucide-vue-next"
import AppModal from "../components/AppModal.vue"
import DonutChart from "../components/DonutChart.vue"
import Treemap from "../components/Treemap.vue"
import { api } from "../api"
import { store } from "../store"
import {
  fmtSize,
  type CleanProgress,
  type CleanResult,
  type DirDetail,
  type DirNode,
  type DriveInfo,
  type SpaceProgress,
  type SpaceScanResult,
} from "../types"

const drives = ref<DriveInfo[]>([])
const selectedDrive = ref("C")

const analyzing = ref(false)
const progress = ref<SpaceProgress | null>(null)
const sessionId = ref(0)
const result = ref<SpaceScanResult | null>(null)

const currentDetail = ref<DirDetail | null>(null)
const loadingDetail = ref(false)

// 右键菜单与删除
const ctxOpen = ref(false)
const ctxX = ref(0)
const ctxY = ref(0)
const ctxNode = ref<DirNode | null>(null)
const confirmDeleteOpen = ref(false)
const deleting = ref(false)
const deleteProgress = ref<CleanProgress | null>(null)
const resultOpen = ref(false)
const deleteResult = ref<CleanResult | null>(null)

let unlisteners: (() => void)[] = []

onMounted(async () => {
  drives.value = await api.listDrives().catch(() => [])
  if (!drives.value.some((d) => d.letter === "C") && drives.value.length > 0) {
    selectedDrive.value = drives.value[0].letter
  }
  unlisteners.push(
    await listen<SpaceProgress>("space-progress", (e) => {
      progress.value = e.payload
    }),
    await listen("space-done", () => {
      analyzing.value = false
      finishScan()
    }),
  )
})

onUnmounted(() => unlisteners.forEach((f) => f()))

async function startAnalyze() {
  result.value = null
  currentDetail.value = null
  analyzing.value = true
  progress.value = { sessionId: 0, dirs: 0, files: 0, bytes: 0 }
  try {
    sessionId.value = await api.startSpaceScan(selectedDrive.value)
  } catch (e) {
    analyzing.value = false
    store.toast(String(e), "error")
  }
}

async function cancelAnalyze() {
  await api.cancelSpaceScan(sessionId.value)
}

async function finishScan() {
  const dto = await api.getSpaceScanResult(sessionId.value).catch(() => null)
  if (!dto) return
  result.value = dto
  await loadDetail(dto.root.path)
}

function crumbs(): { name: string; path: string }[] {
  const path = currentDetail.value?.path ?? ""
  const drive = result.value?.drive ?? selectedDrive.value
  const out = [{ name: `${drive} 盘`, path: `${drive}:\\` }]
  let acc = `${drive}:\\`
  for (const seg of path.slice(3).split("\\")) {
    if (!seg) continue
    acc = acc.endsWith("\\") ? acc + seg : acc + "\\" + seg
    out.push({ name: seg, path: acc })
  }
  return out
}

async function loadDetail(path: string) {
  loadingDetail.value = true
  try {
    currentDetail.value = await api.getDirDetail(sessionId.value, path)
  } catch (e) {
    store.toast(String(e), "error")
  } finally {
    loadingDetail.value = false
  }
}

async function drill(node: DirNode) {
  if (!node.hasChildren) {
    store.toast("该目录没有子目录", "info")
    return
  }
  await loadDetail(node.path)
}

// ---- 右键菜单 ----

function openCtx(payload: { node: DirNode; x: number; y: number }) {
  ctxNode.value = payload.node
  ctxX.value = Math.min(payload.x, window.innerWidth - 190)
  ctxY.value = Math.min(payload.y, window.innerHeight - 140)
  ctxOpen.value = true
}

function ctxClose() {
  ctxOpen.value = false
}

async function ctxOpenDir() {
  const p = ctxNode.value?.path
  ctxClose()
  if (!p) return
  try {
    await api.openPath(p)
  } catch (e) {
    store.toast(String(e), "error")
  }
}

function ctxGoLarge() {
  ctxClose()
  store.go("large")
}

function ctxAskDelete() {
  ctxClose()
  confirmDeleteOpen.value = true
}

async function doDelete() {
  const node = ctxNode.value
  if (!node) return
  confirmDeleteOpen.value = false
  deleting.value = true
  const un = await listen<CleanProgress>("clean-progress", (e) => {
    deleteProgress.value = e.payload
  })
  try {
    deleteResult.value = await api.deleteDirToRecovery(node.path)
    resultOpen.value = true
    store.refreshStats()
    // 会话树不会自动刷新：本地移除该节点，提示可重新分析
    const cur = currentDetail.value
    if (cur) {
      cur.children = cur.children.filter((c) => c.path !== node.path)
    }
  } catch (e) {
    store.toast(String(e), "error")
  } finally {
    un()
    deleting.value = false
  }
}
</script>

<template>
  <div class="space-page">
    <!-- 盘符选择 -->
    <div class="drive-row">
      <button
        v-for="d in drives"
        :key="d.letter"
        class="drive-card card"
        :class="{ active: selectedDrive === d.letter }"
        :disabled="analyzing"
        @click="selectedDrive = d.letter"
      >
        <DonutChart :percent="(d.usedBytes / d.totalBytes) * 100" :size="96" :stroke="10" />
        <div class="drive-info">
          <div class="drive-letter">{{ d.letter }} 盘</div>
          <div class="drive-cap">
            <b>{{ fmtSize(d.usedBytes) }}</b> / {{ fmtSize(d.totalBytes) }}
          </div>
        </div>
      </button>
    </div>

    <!-- 分析进度 -->
    <div v-if="analyzing" class="scan-card card">
      <div class="scanning-row">
        <div class="spinner"></div>
        <div class="scan-progress">
          <div class="progress-text">正在分析 {{ selectedDrive }} 盘目录占用…</div>
          <div class="progress-track">
            <div class="progress-fill indeterminate"></div>
          </div>
          <div class="progress-meta text-muted">
            已遍历 <b>{{ progress?.dirs ?? 0 }}</b> 个目录 · {{ progress?.files ?? 0 }} 个文件 ·
            {{ fmtSize(progress?.bytes ?? 0) }}
          </div>
        </div>
        <button class="btn" @click="cancelAnalyze">取消</button>
      </div>
    </div>

    <!-- 开始分析 -->
    <div v-else-if="!result" class="scan-card card">
      <div class="scan-headline">
        <span class="found-label">
          可视化呈现 <b>{{ selectedDrive }}</b> 盘空间占用：矩形面积 = 目录大小，颜色区分文件类型，点击逐层下钻
        </span>
      </div>
      <button class="btn btn-primary btn-lg" @click="startAnalyze">
        <ScanSearch :size="18" stroke-width="2.2" />
        开始分析
      </button>
    </div>

    <!-- Treemap -->
    <template v-if="result">
      <div class="summary-bar card">
        <div class="summary-item">
          <div class="num">{{ result.drive }} 盘</div>
          <div class="text-muted">已用 {{ fmtSize(result.totalBytes) }}</div>
        </div>
        <div class="summary-item">
          <div class="num">{{ fmtSize(result.totalBytes) }}</div>
          <div class="text-muted">总占用</div>
        </div>
        <div class="summary-item">
          <div class="num">{{ result.fileCount.toLocaleString() }}</div>
          <div class="text-muted">文件数</div>
        </div>
        <div class="summary-item">
          <div class="num">{{ result.dirCount.toLocaleString() }}</div>
          <div class="text-muted">目录数</div>
        </div>
        <button class="btn" @click="startAnalyze">
          <ScanSearch :size="15" stroke-width="2" />
          重新分析
        </button>
      </div>

      <!-- 面包屑 -->
      <div class="crumbs card">
        <template v-for="(c, i) in crumbs()" :key="c.path">
          <span v-if="i > 0" class="sep">›</span>
          <button class="crumb" :class="{ current: i === crumbs().length - 1 }" @click="loadDetail(c.path)">
            {{ c.name }}
          </button>
        </template>
      </div>

      <div v-if="loadingDetail" class="loading-hint text-muted">加载中…</div>
      <Treemap
        v-else-if="currentDetail"
        :nodes="currentDetail.children"
        @drill="drill"
        @context="openCtx"
      />

      <!-- 当前目录详情（PRD 4.3 底部） -->
      <div v-if="currentDetail" class="detail-bar card">
        <div class="detail-path text-muted">{{ currentDetail.path }}</div>
        <div class="detail-stats">
          <b>{{ fmtSize(currentDetail.size) }}</b> · {{ currentDetail.fileCount.toLocaleString() }} 个文件
          <template v-if="result && result.skippedDirs > 0">
            · 跳过受限目录 {{ result.skippedDirs }}
          </template>
        </div>
      </div>
    </template>

    <!-- 右键菜单 -->
    <Teleport to="body">
      <div v-if="ctxOpen" class="ctx-mask" @click="ctxClose" @contextmenu.prevent="ctxClose">
        <div class="ctx-menu card" :style="{ left: ctxX + 'px', top: ctxY + 'px' }">
          <button class="ctx-item" @click="ctxOpenDir">
            <FolderOpen :size="15" stroke-width="2" />
            打开目录
          </button>
          <button class="ctx-item" @click="ctxGoLarge">
            <ScanSearch :size="15" stroke-width="2" />
            去查大文件
          </button>
          <button class="ctx-item danger" @click="ctxAskDelete">
            <Trash2 :size="15" stroke-width="2" />
            删除（移入恢复区）
          </button>
        </div>
      </div>
    </Teleport>

    <!-- 删除确认 -->
    <AppModal :open="confirmDeleteOpen" title="确认删除该目录？" danger @close="confirmDeleteOpen = false">
      <p>
        即将把 <b>{{ ctxNode?.path }}</b>（{{ fmtSize(ctxNode?.size ?? 0) }}）整体移入恢复区，
        保留 7 天，可随时撤销恢复。
      </p>
      <p class="text-muted">系统关键目录与白名单目录受保护，无法删除。</p>
      <template #footer>
        <button class="btn" @click="confirmDeleteOpen = false">取消</button>
        <button class="btn btn-primary" @click="doDelete">移入恢复区</button>
      </template>
    </AppModal>

    <!-- 删除进度 -->
    <AppModal :open="deleting" title="正在移入恢复区…" :closable="false">
      <div class="clean-progress">
        <div class="progress-track">
          <div class="progress-fill indeterminate"></div>
        </div>
        <div class="progress-meta text-muted">
          {{ fmtSize(deleteProgress?.freedBytes ?? 0) }}
          <template v-if="deleteProgress?.skippedLocked">
            · 已跳过占用 {{ deleteProgress?.skippedLocked }} 项
          </template>
        </div>
      </div>
      <template #footer><span></span></template>
    </AppModal>

    <!-- 删除结果 -->
    <AppModal :open="resultOpen" title="已移入恢复区 🎉" @close="resultOpen = false">
      <div v-if="deleteResult" class="result-grid">
        <div class="result-item">
          <div class="num green">{{ fmtSize(deleteResult.freedBytes) }}</div>
          <div class="text-muted">释放空间</div>
        </div>
        <div class="result-item">
          <div class="num">{{ deleteResult.movedCount }}</div>
          <div class="text-muted">移入目录数</div>
        </div>
        <div class="result-item">
          <div class="num">{{ deleteResult.skippedLocked }}</div>
          <div class="text-muted">因占用跳过</div>
        </div>
      </div>
      <p class="text-muted">目录树数据未自动刷新，如需查看最新占用请点击「重新分析」。</p>
      <template #footer>
        <button class="btn" @click="resultOpen = false">关闭</button>
        <button class="btn btn-primary" @click="resultOpen = false; store.go('recovery')">查看恢复区</button>
      </template>
    </AppModal>
  </div>
</template>

<style scoped>
.space-page {
  max-width: 1080px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.drive-row {
  display: flex;
  gap: 14px;
  flex-wrap: wrap;
}

.drive-card {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 14px 18px;
  cursor: pointer;
  border: 2px solid var(--border);
  transition: all 0.15s;
  text-align: left;
}

.drive-card:hover:not(:disabled) {
  border-color: var(--primary);
}

.drive-card.active {
  border-color: var(--primary);
  box-shadow: 0 0 0 3px var(--primary-soft);
}

.drive-letter {
  font-size: 16px;
  font-weight: 700;
}

.drive-cap {
  margin-top: 5px;
  font-size: 13px;
}

.scan-card {
  padding: 24px 26px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
}

.scan-headline .found-label {
  color: var(--text-2);
  font-size: 14px;
}

.scanning-row {
  display: flex;
  align-items: center;
  gap: 18px;
  width: 100%;
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

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.scan-progress {
  flex: 1;
}

.progress-text {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 8px;
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

.summary-bar {
  padding: 16px 22px;
  display: flex;
  align-items: center;
  gap: 28px;
}

.summary-item .num {
  font-size: 18px;
  font-weight: 700;
}

.crumbs {
  padding: 10px 16px;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.sep {
  color: var(--muted);
}

.crumb {
  border: none;
  background: transparent;
  color: var(--primary);
  font-size: 13px;
  cursor: pointer;
  padding: 3px 6px;
  border-radius: 6px;
}

.crumb:hover {
  background: var(--primary-soft);
}

.crumb.current {
  color: var(--text);
  font-weight: 600;
  cursor: default;
}

.loading-hint {
  padding: 30px;
  text-align: center;
}

.detail-bar {
  padding: 12px 18px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  font-size: 13px;
}

.detail-path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ctx-mask {
  position: fixed;
  inset: 0;
  z-index: 60;
}

.ctx-menu {
  position: fixed;
  width: 180px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.ctx-item {
  display: flex;
  align-items: center;
  gap: 9px;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  padding: 9px 10px;
  border-radius: 8px;
  cursor: pointer;
  text-align: left;
}

.ctx-item:hover {
  background: var(--bg);
}

.ctx-item.danger {
  color: var(--danger);
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
