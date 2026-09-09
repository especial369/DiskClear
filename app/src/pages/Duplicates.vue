<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue"
import { listen } from "@tauri-apps/api/event"
import { Cloud, Copy, Sparkles, Trash2 } from "lucide-vue-next"
import AppModal from "../components/AppModal.vue"
import { api } from "../api"
import { store } from "../store"
import { fmtSize, fmtTime, type CleanResult, type DriveInfo, type DupeProgress, type DupeScanResult } from "../types"

const drives = ref<DriveInfo[]>([])
const selectedDrive = ref("C")
const scanning = ref(false)
const progress = ref<DupeProgress | null>(null)
const sessionId = ref(0)
const result = ref<DupeScanResult | null>(null)

const checked = reactive<Record<string, boolean>>({})
const confirmOpen = ref(false)
const confirmAllOpen = ref(false)
const deleting = ref(false)

let unlisteners: (() => void)[] = []

const phaseText = computed(() => {
  const p = progress.value
  if (!p) return ""
  if (p.phase === "walk") return `正在遍历 ${selectedDrive.value} 盘文件（已发现 ${p.done} 个候选）…`
  if (p.phase === "prescreen") return `正在预筛重复组（${p.done}/${p.total} 组）…`
  return `正在精确比对内容（已校验 ${p.done} 个文件）…`
})

const totalWasted = computed(() =>
  (result.value?.groups ?? []).reduce((s, g) => s + g.size * (g.files.length - 1), 0),
)

const selectedPaths = computed(() => {
  const out: string[] = []
  for (const g of result.value?.groups ?? []) {
    g.files.forEach((f, i) => {
      if (i !== g.keepIndex && checked[g.hash + i] && !f.isCloud) out.push(f.path)
    })
  }
  return out
})

const selectedWasted = computed(() => {
  let sum = 0
  for (const g of result.value?.groups ?? []) {
    g.files.forEach((f, i) => {
      if (i !== g.keepIndex && checked[g.hash + i] && !f.isCloud) sum += f.size
    })
  }
  return sum
})

const groupsWithSelection = computed(() =>
  (result.value?.groups ?? []).filter((g) =>
    g.files.some((_, i) => i !== g.keepIndex && checked[g.hash + i] && !g.files[i].isCloud),
  ),
)

onMounted(async () => {
  drives.value = await api.listDrives().catch(() => [])
  unlisteners.push(
    await listen<DupeProgress>("dupe-progress", (e) => {
      progress.value = e.payload
    }),
    await listen("dupe-done", async () => {
      scanning.value = false
      applyResult(await api.getDupeScanResult(sessionId.value).catch(() => null))
    }),
  )
})

onUnmounted(() => unlisteners.forEach((f) => f()))

function applyResult(r: DupeScanResult | null) {
  result.value = r
  if (r) {
    for (const g of r.groups) {
      g.files.forEach((f, i) => {
        checked[g.hash + i] = i !== g.keepIndex && !f.isCloud
      })
    }
  }
}

async function startScan() {
  result.value = null
  scanning.value = true
  progress.value = { sessionId: 0, phase: "walk", done: 0, total: 0, groups: 0 }
  sessionId.value = await api.startDupeScan(selectedDrive.value)
}

async function cancelScan() {
  await api.cancelDupeScan(sessionId.value)
}

function deleteGroupFiles(paths: string[]) {
  return api.deleteDupeFiles(paths)
}

async function doDeleteSelected() {
  confirmOpen.value = false
  deleting.value = true
  try {
    const paths = selectedPaths.value
    const out: CleanResult = await deleteGroupFiles(paths)
    afterDelete(out, paths.length)
  } finally {
    deleting.value = false
  }
}

async function doDeleteAll() {
  confirmAllOpen.value = false
  deleting.value = true
  try {
    const paths: string[] = []
    for (const g of result.value?.groups ?? []) {
      g.files.forEach((f, i) => {
        if (i !== g.keepIndex && !f.isCloud) paths.push(f.path)
      })
    }
    const out: CleanResult = await deleteGroupFiles(paths)
    afterDelete(out, paths.length)
  } finally {
    deleting.value = false
  }
}

function afterDelete(out: CleanResult, requested: number) {
  // 从结果中移除已删除的文件；组内只剩保留项时整组消失
  for (const g of result.value?.groups ?? []) {
    g.files = g.files.filter((f, i) => {
      const wasChecked = i !== g.keepIndex && checked[g.hash + i] && !f.isCloud
      if (wasChecked && out.skippedLocked === 0) return false
      return true
    })
  }
  result.value!.groups = result.value!.groups.filter((g) => g.files.length >= 2)
  for (const g of result.value!.groups) g.keepIndex = 0
  store.toast(
    `已移入恢复区 ${out.movedCount}/${requested} 个文件（释放 ${fmtSize(out.freedBytes)}）${
      out.skippedLocked ? `，${out.skippedLocked} 个因占用跳过` : ""
    }`,
    "success",
  )
  store.refreshStats()
}
</script>

<template>
  <div class="dupe-page">
    <!-- 控制区 -->
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
        <span class="text-muted">按内容精确比对（MD5），1MB 以上文件参与</span>
        <span class="spacer"></span>
        <button v-if="!scanning" class="btn btn-primary" @click="startScan">
          <Copy :size="15" stroke-width="2.2" /> 查找重复文件
        </button>
        <button v-else class="btn" @click="cancelScan">取消</button>
      </div>

      <div v-if="scanning" class="scan-live">
        <div class="spinner"></div>
        <div class="scan-progress">
          <div class="progress-track"><div class="progress-fill indeterminate"></div></div>
          <div class="progress-meta text-muted">{{ phaseText }}</div>
        </div>
      </div>

      <div v-else-if="result" class="summary-row">
        <span>
          发现 <b>{{ result.groups.length }}</b> 组重复文件，可释放
          <b class="green">{{ fmtSize(totalWasted) }}</b>
          <span class="text-muted">（遍历 {{ result.walkedFiles }} 个文件，校验 {{ result.hashedFiles }} 个）</span>
        </span>
        <span class="spacer"></span>
        <button
          v-if="result.groups.length"
          class="btn btn-primary"
          :disabled="deleting"
          @click="confirmAllOpen = true"
        >
          <Sparkles :size="15" /> 一键清理全部冗余副本
        </button>
      </div>
      <div v-if="result?.truncated" class="trunc-note text-muted">
        文件数超出单次比对上限，结果可能不完整（可分盘符扫描）
      </div>
    </div>

    <!-- 空状态 -->
    <div v-if="result && !result.groups.length" class="empty card">
      <div class="empty-title">未发现重复文件 ✨</div>
      <div class="text-muted">{{ selectedDrive }} 盘没有内容相同的多余副本。</div>
    </div>

    <!-- 分组列表 -->
    <div v-for="g in result?.groups ?? []" :key="g.hash" class="group card">
      <div class="group-head">
        <span class="g-size">{{ fmtSize(g.size) }} × {{ g.files.length }} 个副本</span>
        <span class="g-waste">可释放 {{ fmtSize(g.size * (g.files.length - 1)) }}</span>
      </div>
      <div
        v-for="(f, i) in g.files"
        :key="f.path"
        class="file-row"
        :class="{ keep: i === g.keepIndex }"
      >
        <input
          v-if="i !== g.keepIndex"
          v-model="checked[g.hash + i]"
          type="checkbox"
          :disabled="f.isCloud || deleting"
          class="f-check"
        />
        <span v-else class="keep-badge"><Sparkles :size="12" /> 推荐保留</span>
        <div class="f-info">
          <div class="f-name" :title="f.path">
            {{ f.path }}
            <span v-if="f.isCloud" class="cloud-tag"><Cloud :size="12" /> 云</span>
          </div>
          <div class="text-muted">修改于 {{ fmtTime(f.modifiedMs) }}</div>
        </div>
        <span class="f-size">{{ fmtSize(f.size) }}</span>
      </div>
      <div v-if="groupsWithSelection.includes(g)" class="group-foot">
        <button class="btn btn-sm" :disabled="deleting" @click="confirmOpen = true">
          <Trash2 :size="13" /> 删除所选（{{ selectedPaths.length }} 项 / {{ fmtSize(selectedWasted) }}）
        </button>
      </div>
    </div>

    <!-- 删除确认 -->
    <AppModal :open="confirmOpen" title="删除所选重复文件？" @close="confirmOpen = false">
      <p>
        已选择 <b>{{ selectedPaths.length }}</b> 个冗余副本（{{ fmtSize(selectedWasted) }}），将<b>移入恢复区</b>保留
        7 天，期间可随时恢复。
      </p>
      <p class="text-muted">推荐保留项（每个组中最新/最短路径的文件）不会被删除。</p>
      <template #footer>
        <button class="btn" @click="confirmOpen = false">取消</button>
        <button class="btn btn-primary" :disabled="deleting" @click="doDeleteSelected">确认删除</button>
      </template>
    </AppModal>

    <AppModal :open="confirmAllOpen" title="一键清理全部冗余副本？" @close="confirmAllOpen = false">
      <p>
        将删除 <b>{{ result?.groups.length }}</b> 组中的全部冗余副本，预计释放
        <b class="green">{{ fmtSize(totalWasted) }}</b>。
      </p>
      <p class="text-muted">每组的推荐保留项会保留；所有删除均移入恢复区，可随时撤销。云占位文件会自动跳过。</p>
      <template #footer>
        <button class="btn" @click="confirmAllOpen = false">取消</button>
        <button class="btn btn-primary" :disabled="deleting" @click="doDeleteAll">开始清理</button>
      </template>
    </AppModal>
  </div>
</template>

<style scoped>
.dupe-page {
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

.spacer {
  flex: 1;
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

.green {
  color: var(--accent);
  font-size: 15px;
}

.trunc-note {
  margin-top: 10px;
  font-size: 12px;
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

.group {
  overflow: hidden;
}

.group-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 18px;
  background: var(--bg);
  border-bottom: 1px solid var(--border);
}

.g-size {
  font-weight: 700;
  font-size: 13.5px;
}

.g-waste {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--accent);
  background: var(--accent-soft);
  border-radius: 99px;
  padding: 3px 12px;
}

.file-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 18px 10px 14px;
  border-bottom: 1px dashed var(--border);
}

.file-row.keep {
  background: var(--accent-soft);
}

.file-row:last-of-type {
  border-bottom: none;
}

.f-check {
  width: 16px;
  height: 16px;
  accent-color: var(--primary);
  flex-shrink: 0;
}

.keep-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  font-weight: 700;
  color: var(--accent);
  flex-shrink: 0;
  width: 88px;
}

.f-info {
  flex: 1;
  min-width: 0;
}

.f-name {
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  display: flex;
  align-items: center;
  gap: 8px;
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

.f-size {
  font-weight: 600;
  font-size: 13px;
  flex-shrink: 0;
}

.group-foot {
  padding: 10px 18px;
  display: flex;
  justify-content: flex-end;
  border-top: 1px solid var(--border);
}

.btn-sm {
  padding: 6px 12px;
  font-size: 12px;
}
</style>
