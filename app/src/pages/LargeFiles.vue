<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue"
import { listen } from "@tauri-apps/api/event"
import { Cloud, EyeOff, FolderOpen, Scan, Search, Trash2, TriangleAlert } from "lucide-vue-next"
import AppModal from "../components/AppModal.vue"
import { api } from "../api"
import { store } from "../store"
import {
  fmtSize,
  fmtTime,
  fileType,
  RECOVERY_MAX_FILE_BYTES,
  type DriveInfo,
  type LargeDeleteOutcome,
  type LargeFile,
  type LargeProgress,
  type LargeScanResult,
  type Settings,
} from "../types"

const drives = ref<DriveInfo[]>([])
const selectedDrive = ref("C")
const thresholdMb = ref(100)
const settings = ref<Settings | null>(null)

const scanning = ref(false)
const progress = ref<LargeProgress | null>(null)
const sessionId = ref(0)
const result = ref<LargeScanResult | null>(null)
const filter = ref("")

const target = ref<LargeFile | null>(null)
const permanentAck = ref(false)
const deleteBusy = ref(false)

let unlisteners: (() => void)[] = []

const ignoredCount = computed(() => settings.value?.ignoredFiles.length ?? 0)

const filtered = computed(() => {
  const kw = filter.value.trim().toLowerCase()
  const list = kw
    ? (result.value?.files ?? []).filter((f) => f.path.toLowerCase().includes(kw))
    : result.value?.files ?? []
  return list.slice(0, 500)
})

onMounted(async () => {
  settings.value = await api.getSettings().catch(() => null)
  thresholdMb.value = settings.value?.largeFileThresholdMb ?? 100
  drives.value = await api.listDrives().catch(() => [])
  unlisteners.push(
    await listen<LargeProgress>("large-progress", (e) => {
      progress.value = e.payload
    }),
    await listen("large-done", async () => {
      scanning.value = false
      result.value = await api.getLargeScanResult(sessionId.value).catch(() => null)
    }),
  )
})

onUnmounted(() => unlisteners.forEach((f) => f()))

async function startScan() {
  result.value = null
  scanning.value = true
  progress.value = { sessionId: 0, files: 0, bytes: 0 }
  sessionId.value = await api.startLargeScan(selectedDrive.value, thresholdMb.value)
}

async function cancelScan() {
  await api.cancelLargeScan(sessionId.value)
}

function askDelete(f: LargeFile) {
  target.value = f
  permanentAck.value = false
}

const needsPermanent = computed(() => (target.value?.size ?? 0) > RECOVERY_MAX_FILE_BYTES)

async function doDelete() {
  const f = target.value
  if (!f) return
  deleteBusy.value = true
  try {
    const out: LargeDeleteOutcome = await api.deleteLargeFile(f.path, needsPermanent.value)
    if (out.mode === "permanent") {
      store.toast(`已永久删除（${fmtSize(out.freedBytes)}）`, "success")
    } else {
      store.toast(`已移入恢复区（${fmtSize(out.freedBytes)}），可随时撤销`, "success")
    }
    result.value!.files = result.value!.files.filter((x) => x.path !== f.path)
    store.refreshStats()
    target.value = null
  } catch (e) {
    store.toast(String(e), "error")
  } finally {
    deleteBusy.value = false
  }
}

async function ignore(f: LargeFile) {
  settings.value = await api.addIgnoredFile(f.path)
  result.value!.files = result.value!.files.filter((x) => x.path !== f.path)
  store.toast("已加入忽略列表，扫描时将不再显示", "success")
}

async function clearIgnored() {
  settings.value = await api.clearIgnoredFiles()
  store.toast("忽略列表已清空（重新扫描后生效）", "success")
}
</script>

<template>
  <div class="large-page">
    <!-- 扫描控制 -->
    <div class="ctrl card">
      <div class="ctrl-row">
        <div class="drive-chips">
          <button
            v-for="d in drives"
            :key="d.letter"
            class="chip"
            :class="{ active: selectedDrive === d.letter }"
            :disabled="scanning"
            @click="selectedDrive = d.letter"
          >
            {{ d.letter }} 盘
          </button>
        </div>
        <select v-model.number="thresholdMb" :disabled="scanning">
          <option :value="50">大于 50 MB</option>
          <option :value="100">大于 100 MB</option>
          <option :value="500">大于 500 MB</option>
          <option :value="1024">大于 1 GB</option>
        </select>
        <button v-if="!scanning" class="btn btn-primary" @click="startScan">
          <Scan :size="15" stroke-width="2.2" /> 扫描大文件
        </button>
        <button v-else class="btn" @click="cancelScan">取消</button>
      </div>

      <div v-if="scanning" class="scan-live">
        <div class="spinner"></div>
        <div class="scan-progress">
          <div class="progress-track"><div class="progress-fill indeterminate"></div></div>
          <div class="progress-meta text-muted">
            已遍历 <b>{{ progress?.files ?? 0 }}</b> 个文件 · {{ fmtSize(progress?.bytes ?? 0) }}
          </div>
        </div>
      </div>

      <div v-else-if="result" class="summary-row">
        <span>
          {{ result.drive }} 盘共 <b>{{ result.files.length }}</b> 个大于
          {{ thresholdMb }}MB 的文件，合计
          <b class="blue">{{ fmtSize(result.totalBytes) }}</b>
        </span>
        <span v-if="result.truncated" class="trunc-note">
          <TriangleAlert :size="13" /> 结果过多，仅显示最大的 {{ result.files.length }} 项
        </span>
        <span class="spacer"></span>
        <span v-if="ignoredCount" class="ignored-note text-muted">
          忽略列表 {{ ignoredCount }} 项
          <button class="link-btn" @click="clearIgnored">清空</button>
        </span>
      </div>
    </div>

    <!-- 过滤 -->
    <div v-if="result && result.files.length" class="filter-row">
      <div class="search-box">
        <Search :size="15" class="search-icon" />
        <input v-model="filter" type="text" placeholder="按路径过滤（如 .iso、Downloads）" />
      </div>
      <span class="text-muted">显示前 {{ filtered.length }} 项（按大小降序）</span>
    </div>

    <!-- 空状态 -->
    <div v-if="result && !result.files.length" class="empty card">
      <div class="empty-title">未发现大于 {{ thresholdMb }}MB 的文件 🎉</div>
      <div class="text-muted">试试调低阈值，或换一个盘符扫描。</div>
    </div>

    <!-- 结果表 -->
    <div v-if="result && result.files.length" class="list card">
      <div v-for="f in filtered" :key="f.path" class="row">
        <div class="row-main">
          <div class="row-name">
            <span :title="f.path">{{ f.path.split("\\").pop() }}</span>
            <span v-if="f.isCloud" class="cloud-tag" title="OneDrive 云端占位文件，删除将影响云端">
              <Cloud :size="13" /> 云
            </span>
          </div>
          <div class="row-path text-muted" :title="f.path">{{ f.path }}</div>
        </div>
        <span class="type-badge" :class="fileType(f.path).cls">{{ fileType(f.path).label }}</span>
        <span class="mtime text-muted">{{ fmtTime(f.modifiedMs) }}</span>
        <span class="size">{{ fmtSize(f.size) }}</span>
        <div class="row-actions">
          <button class="icon-btn" title="打开所在文件夹" @click="api.openPath(f.path)">
            <FolderOpen :size="15" />
          </button>
          <button class="icon-btn" title="加入忽略列表" @click="ignore(f)">
            <EyeOff :size="15" />
          </button>
          <button class="icon-btn danger" title="删除" @click="askDelete(f)">
            <Trash2 :size="15" />
          </button>
        </div>
      </div>
    </div>

    <!-- 删除确认（分级：恢复区 / 永久删除） -->
    <AppModal
      :open="!!target"
      :title="needsPermanent ? '危险：此文件将永久删除' : '删除此文件？'"
      :danger="needsPermanent"
      @close="target = null"
    >
      <template v-if="target">
        <p class="del-path" :title="target.path">{{ target.path }}</p>

        <template v-if="!needsPermanent">
          <p>
            文件大小 <b>{{ fmtSize(target.size) }}</b>，将<b>移入恢复区</b>保留 7 天，期间可随时恢复到原位置。
          </p>
          <p v-if="target.isCloud" class="cloud-warn">
            <TriangleAlert :size="13" /> 这是 OneDrive 云端占位文件，恢复区移入不会下载内容，可放心操作。
          </p>
        </template>

        <template v-else>
          <p>
            文件大小 <b>{{ fmtSize(target.size) }}</b>，超过恢复区单文件上限（5GB），<b class="red">无法撤销恢复</b>。
          </p>
          <p v-if="target.isCloud" class="cloud-warn">
            <TriangleAlert :size="13" /> 这是 OneDrive 云端占位文件，永久删除会影响云端副本（将在下次同步时从云端移除）。
          </p>
          <label class="ack">
            <input v-model="permanentAck" type="checkbox" />
            我已了解该文件将被<b class="red">永久删除且无法恢复</b>
          </label>
        </template>
      </template>
      <template #footer>
        <button class="btn" @click="target = null">取消</button>
        <button
          class="btn btn-primary"
          :class="{ 'btn-danger-solid': needsPermanent }"
          :disabled="deleteBusy || (needsPermanent && !permanentAck)"
          @click="doDelete"
        >
          {{ needsPermanent ? "永久删除" : "移入恢复区" }}
        </button>
      </template>
    </AppModal>
  </div>
</template>

<style scoped>
.large-page {
  max-width: 1080px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.ctrl {
  padding: 18px 22px;
}

.ctrl-row {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;
}

.drive-chips {
  display: flex;
  gap: 8px;
}

.chip {
  border: 1.5px solid var(--border);
  background: var(--card);
  color: var(--text-2);
  border-radius: 99px;
  padding: 6px 18px;
  font-size: 13px;
  cursor: pointer;
  transition: all 0.15s;
}

.chip.active {
  border-color: var(--primary);
  color: var(--primary);
  background: var(--primary-soft);
  font-weight: 600;
}

.scan-live {
  display: flex;
  align-items: center;
  gap: 16px;
  margin-top: 16px;
}

.spinner {
  width: 20px;
  height: 20px;
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
  margin-top: 7px;
}

.summary-row {
  margin-top: 14px;
  display: flex;
  align-items: center;
  gap: 14px;
  font-size: 13px;
  flex-wrap: wrap;
}

.blue {
  color: var(--primary);
  font-size: 15px;
}

.trunc-note {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--warn);
  font-size: 12px;
}

.spacer {
  flex: 1;
}

.ignored-note {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.link-btn {
  border: none;
  background: none;
  color: var(--primary);
  cursor: pointer;
  font-size: 12px;
  padding: 0;
}

.filter-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
}

.search-box {
  position: relative;
  flex: 1;
  max-width: 420px;
}

.search-icon {
  position: absolute;
  left: 10px;
  top: 50%;
  transform: translateY(-50%);
  color: var(--muted);
}

.search-box input {
  width: 100%;
  padding-left: 32px;
}

.empty {
  padding: 52px;
  text-align: center;
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: center;
}

.empty-title {
  font-size: 16px;
  font-weight: 700;
}

.list {
  overflow: hidden;
}

.row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 11px 18px;
  border-bottom: 1px dashed var(--border);
}

.row:hover {
  background: var(--bg);
}

.row:last-child {
  border-bottom: none;
}

.row-main {
  flex: 1;
  min-width: 0;
}

.row-name {
  font-size: 13.5px;
  font-weight: 600;
  display: flex;
  align-items: center;
  gap: 8px;
}

.row-name span:first-child {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.cloud-tag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: 11px;
  color: #0284c7;
  background: #e0f2fe;
  border-radius: 99px;
  padding: 1px 8px;
  flex-shrink: 0;
}

.row-path {
  margin-top: 2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.type-badge {
  font-size: 11px;
  font-weight: 600;
  border-radius: 99px;
  padding: 3px 10px;
  flex-shrink: 0;
}

.t-video { color: #7c3aed; background: #f3e8ff; }
.t-image { color: #db2777; background: #fce7f3; }
.t-audio { color: #0d9488; background: #ccfbf1; }
.t-archive { color: #d97706; background: #fef3c7; }
.t-image-disk { color: #dc2626; background: #fee2e2; }
.t-installer { color: #059669; background: #d1fae5; }
.t-doc { color: #2563eb; background: #dbeafe; }
.t-other { color: var(--text-2); background: var(--bg); }

.mtime {
  width: 118px;
  flex-shrink: 0;
  text-align: right;
}

.size {
  width: 92px;
  flex-shrink: 0;
  text-align: right;
  font-weight: 700;
  color: var(--primary);
}

.row-actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}

.icon-btn {
  border: none;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  padding: 6px;
  border-radius: 7px;
  display: flex;
}

.icon-btn:hover {
  color: var(--primary);
  background: var(--primary-soft);
}

.icon-btn.danger:hover {
  color: var(--danger);
  background: var(--danger-soft);
}

.del-path {
  word-break: break-all;
  background: var(--bg);
  border-radius: 8px;
  padding: 9px 12px;
  font-size: 12px;
  margin-bottom: 10px;
}

.red {
  color: var(--danger);
}

.cloud-warn {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--warn);
  background: var(--warn-soft);
  border-radius: 8px;
  padding: 8px 11px;
  font-size: 12px;
  margin-top: 8px;
}

.ack {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
  font-size: 13px;
  cursor: pointer;
  user-select: none;
}

.ack input {
  width: 16px;
  height: 16px;
  accent-color: var(--danger);
}

.btn-danger-solid {
  background: var(--danger) !important;
  border-color: var(--danger) !important;
}
</style>
