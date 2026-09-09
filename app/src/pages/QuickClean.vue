<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue"
import { listen } from "@tauri-apps/api/event"
import { Scan, ShieldAlert, TriangleAlert } from "lucide-vue-next"
import DonutChart from "../components/DonutChart.vue"
import AppModal from "../components/AppModal.vue"
import { api } from "../api"
import { store } from "../store"
import { fmtSize, type CategoryResult, type CleanProgress, type CleanResult, type DriveInfo, type EnvInfo, type ScanProgress } from "../types"

const drives = ref<DriveInfo[]>([])
const selectedDrive = ref<string>("C")
const env = ref<EnvInfo | null>(null)

const scanning = ref(false)
const cleaning = ref(false)
const progressText = ref("")
const progressFiles = ref(0)
const progressSize = ref(0)

const results = ref<CategoryResult[]>([])
const selected = reactive<Record<string, boolean>>({})
const sessionId = ref(0)

const confirmOpen = ref(false)
const resultOpen = ref(false)
const cleanResult = ref<CleanResult | null>(null)
let cleanProgressTimer: number | null = null
const cleanProgress = ref<CleanProgress | null>(null)

let unlisteners: (() => void)[] = []

const totalFound = computed(() => results.value.reduce((s, r) => s + r.totalSize, 0))
const totalSelected = computed(() =>
  results.value.filter((r) => selected[r.id] && !r.disabled).reduce((s, r) => s + r.totalSize, 0),
)
const anyWarning = computed(() => results.value.some((r) => r.warning && !r.disabled))

onMounted(async () => {
  env.value = await api.getEnvInfo().catch(() => null)
  drives.value = await api.listDrives().catch(() => [])
  if (!drives.value.some((d) => d.letter === "C") && drives.value.length > 0) {
    selectedDrive.value = drives.value[0].letter
  }
  unlisteners.push(
    await listen<ScanProgress>("scan-progress", (e) => {
      progressText.value = `正在扫描：${e.payload.categoryName}（${e.payload.categoryIndex}/${e.payload.totalCategories}）`
      progressFiles.value = e.payload.foundFiles
      progressSize.value = e.payload.foundSize
    }),
    await listen("scan-done", () => {
      scanning.value = false
      finishScan()
    }),
  )
})

onUnmounted(() => unlisteners.forEach((f) => f()))

async function startScan() {
  results.value = []
  scanning.value = true
  progressText.value = "正在准备扫描…"
  progressFiles.value = 0
  progressSize.value = 0
  try {
    sessionId.value = await api.startScan(selectedDrive.value)
  } catch (e) {
    scanning.value = false
    store.toast(String(e), "error")
  }
}

async function cancelScan() {
  await api.cancelScan(sessionId.value)
}

async function finishScan() {
  const dto = await api.getScanResult(sessionId.value).catch(() => null)
  if (!dto) return
  results.value = dto.results
  for (const r of dto.results) {
    selected[r.id] = !r.disabled && r.fileCount > 0
  }
}

async function doClean() {
  confirmOpen.value = false
  cleaning.value = true
  cleanProgress.value = { done: 0, total: 1, freedBytes: 0, skippedLocked: 0 }
  cleanProgressTimer = window.setInterval(async () => {
    // 进度由事件驱动；此处兜底刷新
  }, 500)
  const un = await listen<CleanProgress>("clean-progress", (e) => {
    cleanProgress.value = e.payload
  })
  try {
    const ids = Object.keys(selected).filter((k) => selected[k])
    cleanResult.value = await api.clean(sessionId.value, ids)
    resultOpen.value = true
    store.refreshStats()
  } catch (e) {
    store.toast(String(e), "error")
  } finally {
    un()
    if (cleanProgressTimer) window.clearInterval(cleanProgressTimer)
    cleaning.value = false
  }
}
</script>

<template>
  <div class="quick-clean">
    <!-- 管理员提示 -->
    <div v-if="env && !env.isAdmin" class="admin-banner">
      <ShieldAlert :size="15" stroke-width="2" />
      当前未以管理员身份运行，系统目录清理将不完整（右键"以管理员身份运行"可深度清理）
    </div>

    <!-- 盘符选择 -->
    <div class="drive-row">
      <button
        v-for="d in drives"
        :key="d.letter"
        class="drive-card card"
        :class="{ active: selectedDrive === d.letter }"
        @click="!scanning && (selectedDrive = d.letter)"
      >
        <DonutChart :percent="(d.usedBytes / d.totalBytes) * 100" :size="104" :stroke="10" />
        <div class="drive-info">
          <div class="drive-letter">{{ d.letter }} 盘</div>
          <div class="drive-cap">
            <b>{{ fmtSize(d.usedBytes) }}</b> / {{ fmtSize(d.totalBytes) }}
          </div>
          <div class="drive-free text-muted">剩余 {{ fmtSize(d.freeBytes) }} · {{ d.fsName }}</div>
        </div>
      </button>
    </div>

    <!-- 扫描区 -->
    <div class="scan-card card">
      <template v-if="!scanning">
        <div class="scan-headline">
          <template v-if="results.length">
            <span class="found-size">{{ fmtSize(totalFound) }}</span>
            <span class="found-label">可清理空间（{{ selectedDrive }} 盘）</span>
          </template>
          <template v-else>
            <span class="found-label">
              扫描 <b>{{ selectedDrive }}</b> 盘垃圾文件：临时文件、回收站、浏览器缓存、更新缓存、系统日志、缩略图
            </span>
          </template>
        </div>
        <button class="btn btn-primary btn-lg" :disabled="cleaning" @click="startScan">
          <Scan :size="18" stroke-width="2.2" />
          {{ results.length ? "重新扫描" : "立即扫描" }}
        </button>
      </template>

      <template v-else>
        <div class="scanning-row">
          <div class="spinner"></div>
          <div class="scan-progress">
            <div class="progress-text">{{ progressText }}</div>
            <div class="progress-track">
              <div class="progress-fill indeterminate"></div>
            </div>
            <div class="progress-meta text-muted">
              已发现 <b>{{ progressFiles }}</b> 个文件 · {{ fmtSize(progressSize) }}
            </div>
          </div>
          <button class="btn" @click="cancelScan">取消</button>
        </div>
      </template>
    </div>

    <!-- 分类结果 -->
    <div v-if="results.length" class="cat-list">
      <label
        v-for="r in results"
        :key="r.id"
        class="cat-item card"
        :class="{ disabled: r.disabled, checked: selected[r.id] && !r.disabled }"
      >
        <input
          v-model="selected[r.id]"
          type="checkbox"
          :disabled="r.disabled || r.fileCount === 0"
          class="cat-check"
        />
        <div class="cat-main">
          <div class="cat-title-row">
            <span class="cat-name">{{ r.name }}</span>
            <span class="risk-badge" :class="r.risk === 'medium' ? 'risk-medium' : 'risk-low'">
              {{ r.risk === "medium" ? "中风险" : "低风险" }}
            </span>
            <span v-if="r.needsAdmin" class="admin-tag">需管理员</span>
          </div>
          <div class="cat-desc text-muted">{{ r.desc }}</div>
          <div v-if="r.warning" class="cat-warning">
            <TriangleAlert :size="13" stroke-width="2" />
            {{ r.warning }}
          </div>
        </div>
        <div class="cat-size">
          <div class="size-num">{{ fmtSize(r.totalSize) }}</div>
          <div class="text-muted">{{ r.fileCount }} 个文件</div>
        </div>
      </label>

      <!-- 清理栏 -->
      <div class="clean-bar card">
        <div class="clean-info">
          已选择 <b>{{ Object.keys(selected).filter((k) => selected[k] && !results.find((r) => r.id === k)?.disabled).length }}</b> 类
          · 预计释放 <b class="green">{{ fmtSize(totalSelected) }}</b>
        </div>
        <button class="btn btn-primary btn-lg" :disabled="totalSelected === 0 || cleaning" @click="confirmOpen = true">
          立即清理
        </button>
      </div>
      <div v-if="anyWarning" class="warning-note">
        <TriangleAlert :size="13" stroke-width="2" />
        带提示的分类中，运行中被锁定的文件会自动跳过并计入"跳过"数量，不会影响其他文件。
      </div>
    </div>

    <!-- 清理确认 -->
    <AppModal open-title="确认清理" :open="confirmOpen" title="确认清理所选项目？" @close="confirmOpen = false">
      <p>
        即将清理 <b>{{ fmtSize(totalSelected) }}</b>。所有文件将移入恢复区保留
        <b>{{ 7 }} 天</b>，期间可随时恢复，不会直接永久删除。
      </p>
      <p class="text-muted">提示：清理浏览器缓存后，首次打开网页会稍慢（缓存重建）。</p>
      <template #footer>
        <button class="btn" @click="confirmOpen = false">取消</button>
        <button class="btn btn-primary" @click="doClean">确认清理</button>
      </template>
    </AppModal>

    <!-- 清理进度 -->
    <AppModal :open="cleaning" title="正在清理…" :closable="false">
      <div class="clean-progress">
        <div class="progress-track">
          <div
            class="progress-fill"
            :style="{ width: cleanProgress ? (cleanProgress.done / Math.max(1, cleanProgress.total)) * 100 + '%' : '0%' }"
          ></div>
        </div>
        <div class="progress-meta text-muted">
          {{ cleanProgress?.done ?? 0 }} / {{ cleanProgress?.total ?? 0 }} 个文件 · 已释放
          {{ fmtSize(cleanProgress?.freedBytes ?? 0) }}
          <template v-if="cleanProgress?.skippedLocked">
            · 已跳过占用 {{ cleanProgress?.skippedLocked }} 个
          </template>
        </div>
      </div>
      <template #footer><span></span></template>
    </AppModal>

    <!-- 清理结果 -->
    <AppModal :open="resultOpen" title="清理完成 🎉" @close="resultOpen = false">
      <div v-if="cleanResult" class="result-grid">
        <div class="result-item">
          <div class="num green">{{ fmtSize(cleanResult.freedBytes) }}</div>
          <div class="text-muted">已释放空间</div>
        </div>
        <div class="result-item">
          <div class="num">{{ cleanResult.movedCount }}</div>
          <div class="text-muted">移入恢复区文件</div>
        </div>
        <div class="result-item">
          <div class="num">{{ cleanResult.skippedLocked }}</div>
          <div class="text-muted">因占用跳过</div>
        </div>
      </div>
      <p class="text-muted">
        文件保存在恢复区（7 天后自动清除），可在底部状态栏「恢复区」中查看或恢复。
      </p>
      <template #footer>
        <button class="btn" @click="resultOpen = false">关闭</button>
        <button class="btn btn-primary" @click="resultOpen = false; store.go('recovery')">查看恢复区</button>
      </template>
    </AppModal>
  </div>
</template>

<style scoped>
.quick-clean {
  max-width: 980px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.admin-banner {
  display: flex;
  align-items: center;
  gap: 8px;
  background: var(--warn-soft);
  color: var(--warn);
  font-size: 12px;
  padding: 9px 14px;
  border-radius: 10px;
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
  padding: 16px 20px;
  cursor: pointer;
  border: 2px solid var(--border);
  transition: all 0.15s;
  text-align: left;
}

.drive-card:hover {
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

.drive-free {
  margin-top: 3px;
}

.scan-card {
  padding: 26px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
}

.scan-headline .found-size {
  font-size: 34px;
  font-weight: 800;
  color: var(--primary);
  margin-right: 12px;
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

.cat-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.cat-item {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 15px 18px;
  cursor: pointer;
  border: 1.5px solid var(--border);
  transition: all 0.15s;
}

.cat-item.checked {
  border-color: var(--primary);
}

.cat-item.disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.cat-check {
  width: 17px;
  height: 17px;
  accent-color: var(--primary);
  flex-shrink: 0;
}

.cat-main {
  flex: 1;
  min-width: 0;
}

.cat-title-row {
  display: flex;
  align-items: center;
  gap: 9px;
}

.cat-name {
  font-size: 14px;
  font-weight: 600;
}

.admin-tag {
  font-size: 11px;
  color: var(--muted);
  border: 1px solid var(--border);
  padding: 1px 7px;
  border-radius: 99px;
}

.cat-desc {
  margin-top: 3px;
}

.cat-warning {
  margin-top: 7px;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--warn);
  background: var(--warn-soft);
  border-radius: 7px;
  padding: 5px 9px;
  width: fit-content;
}

.cat-size {
  text-align: right;
  flex-shrink: 0;
}

.size-num {
  font-size: 16px;
  font-weight: 700;
  color: var(--primary);
}

.clean-bar {
  margin-top: 8px;
  padding: 16px 22px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  position: sticky;
  bottom: 0;
}

.green {
  color: var(--accent);
}

.warning-note {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--muted);
  font-size: 12px;
  padding: 0 4px;
}

.clean-progress {
  padding: 6px 0;
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
