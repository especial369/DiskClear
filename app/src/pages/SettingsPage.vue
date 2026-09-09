<script setup lang="ts">
import { onMounted, reactive, ref } from "vue"
import { Plus, X } from "lucide-vue-next"
import { api } from "../api"
import { applyTheme, store } from "../store"
import type { EnvInfo, Settings } from "../types"

const s = reactive<Settings>({
  recoveryRetentionDays: 7,
  whitelistDirs: [],
  fat32Policy: "skip",
  largeFileThresholdMb: 100,
  scanThreads: 4,
  theme: "system",
  ignoredFiles: [],
})
const env = ref<EnvInfo | null>(null)
const newDir = ref("")
const saving = ref(false)

onMounted(async () => {
  Object.assign(s, await api.getSettings())
  env.value = await api.getEnvInfo().catch(() => null)
})

async function save() {
  saving.value = true
  try {
    const saved = await api.saveSettings({ ...s })
    Object.assign(s, saved)
    applyTheme(s.theme)
    store.toast("设置已保存", "success")
  } catch (e) {
    store.toast(String(e), "error")
  } finally {
    saving.value = false
  }
}

function addDir() {
  const d = newDir.value.trim()
  if (!d) return
  if (s.whitelistDirs.some((x) => x.toLowerCase() === d.toLowerCase())) {
    store.toast("该目录已在白名单中", "info")
    return
  }
  s.whitelistDirs.push(d)
  newDir.value = ""
}

function removeDir(i: number) {
  s.whitelistDirs.splice(i, 1)
}
</script>

<template>
  <div class="settings-page">
    <div class="section card">
      <div class="section-title">安全</div>

      <div class="row">
        <div class="row-info">
          <div class="row-name">恢复区保留天数</div>
          <div class="row-desc text-muted">被清理文件在恢复区保留的时间（1-30 天），到期自动清除</div>
        </div>
        <select v-model.number="s.recoveryRetentionDays">
          <option v-for="d in [1, 3, 7, 14, 30]" :key="d" :value="d">{{ d }} 天</option>
        </select>
      </div>

      <div class="row">
        <div class="row-info">
          <div class="row-name">FAT32 超 4GB 文件策略</div>
          <div class="row-desc text-muted">
            FAT32 盘上超过 4GB 的文件无法移入恢复区时的处理方式
          </div>
        </div>
        <select v-model="s.fat32Policy">
          <option value="skip">跳过并提示（推荐）</option>
          <option value="confirm_delete">增强确认后直接删除</option>
        </select>
      </div>

      <div class="row col">
        <div class="row-info">
          <div class="row-name">白名单目录</div>
          <div class="row-desc text-muted">扫描与清理时始终跳过这些目录</div>
        </div>
        <div class="wl-list">
          <div v-for="(d, i) in s.whitelistDirs" :key="d" class="wl-item">
            <span class="wl-path">{{ d }}</span>
            <button class="wl-remove" @click="removeDir(i)"><X :size="13" /></button>
          </div>
          <div class="wl-add">
            <input
              v-model="newDir"
              type="text"
              placeholder="输入目录路径，如 D:\我的资料"
              @keydown.enter="addDir"
            />
            <button class="btn" @click="addDir"><Plus :size="14" /> 添加</button>
          </div>
        </div>
      </div>
    </div>

    <div class="section card">
      <div class="section-title">扫描</div>
      <div class="row">
        <div class="row-info">
          <div class="row-name">大文件阈值</div>
          <div class="row-desc text-muted">大文件查找页使用的判定阈值（M2 功能）</div>
        </div>
        <select v-model.number="s.largeFileThresholdMb">
          <option :value="50">50 MB</option>
          <option :value="100">100 MB</option>
          <option :value="500">500 MB</option>
          <option :value="1024">1 GB</option>
        </select>
      </div>
      <div class="row">
        <div class="row-info">
          <div class="row-name">扫描线程数</div>
          <div class="row-desc text-muted">全盘扫描并发数（M2 功能）</div>
        </div>
        <select v-model.number="s.scanThreads">
          <option v-for="n in [1, 2, 4, 8]" :key="n" :value="n">{{ n }}</option>
        </select>
      </div>
    </div>

    <div class="section card">
      <div class="section-title">外观</div>
      <div class="row">
        <div class="row-info">
          <div class="row-name">主题</div>
          <div class="row-desc text-muted">跟随系统时自动适配深浅色</div>
        </div>
        <select v-model="s.theme" @change="applyTheme(s.theme)">
          <option value="system">跟随系统</option>
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </div>
    </div>

    <div class="save-row">
      <button class="btn btn-primary btn-lg" :disabled="saving" @click="save">
        {{ saving ? "保存中…" : "保存设置" }}
      </button>
    </div>

    <div class="about text-muted">
      DiskClear 盘清 v{{ env?.version ?? "?" }} · 隐私承诺：所有扫描与清理在本地完成，不上传任何文件数据，无联网依赖
    </div>
  </div>
</template>

<style scoped>
.settings-page {
  max-width: 760px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.section {
  padding: 20px 24px;
}

.section-title {
  font-size: 15px;
  font-weight: 700;
  margin-bottom: 6px;
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 13px 0;
  border-bottom: 1px dashed var(--border);
}

.row:last-child {
  border-bottom: none;
}

.row.col {
  flex-direction: column;
  align-items: stretch;
}

.row-name {
  font-size: 13.5px;
  font-weight: 600;
}

.row-desc {
  margin-top: 3px;
}

select {
  min-width: 200px;
}

.wl-list {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 7px;
}

.wl-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: var(--bg);
  border-radius: 8px;
  padding: 8px 12px;
}

.wl-path {
  font-size: 12.5px;
}

.wl-remove {
  border: none;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  padding: 3px;
  border-radius: 6px;
  display: flex;
}

.wl-remove:hover {
  color: var(--danger);
  background: var(--danger-soft);
}

.wl-add {
  display: flex;
  gap: 8px;
}

.wl-add input {
  flex: 1;
}

.save-row {
  display: flex;
  justify-content: flex-end;
}

.about {
  text-align: center;
  padding: 6px 0 14px;
}
</style>
