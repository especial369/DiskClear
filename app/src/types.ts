// 与 Rust serde (rename_all = camelCase) 对齐的 IPC 类型
export interface DriveInfo {
  letter: string
  totalBytes: number
  freeBytes: number
  usedBytes: number
  fsName: string
  driveType: string
}

export interface CategoryResult {
  id: string
  name: string
  desc: string
  risk: string
  fileCount: number
  totalSize: number
  needsAdmin: boolean
  disabled: boolean
  warning: string | null
}

export interface ScanResultDto {
  sessionId: number
  drive: string
  done: boolean
  cancelled: boolean
  results: CategoryResult[]
}

export interface ScanProgress {
  sessionId: number
  categoryIndex: number
  totalCategories: number
  categoryName: string
  foundFiles: number
  foundSize: number
  cancelled?: boolean
}

export interface CleanProgress {
  done: number
  total: number
  freedBytes: number
  skippedLocked: number
  finished?: boolean
}

export interface CleanResult {
  batchId: string
  freedBytes: number
  movedCount: number
  skippedLocked: number
  skippedMissing: number
  rejectedUnsafe: number
  errors: string[]
  recoveryBytes: number
}

export interface RecoveryEntry {
  id: string
  batchId: string
  originalPath: string
  recoveryPath: string
  size: number
  deletedAtMs: number
  expiresAtMs: number
  categoryId: string
  drive: string
}

export interface Settings {
  recoveryRetentionDays: number
  whitelistDirs: string[]
  fat32Policy: string
  largeFileThresholdMb: number
  scanThreads: number
  theme: string
  ignoredFiles: string[]
}

export interface EnvInfo {
  isAdmin: boolean
  browsersRunning: string[]
  wuauservRunning: boolean
  version: string
}

export interface Stats {
  freedTotalBytes: number
  recoveryBytes: number
}

// ---------- M2 ----------

export interface LargeFile {
  path: string
  size: number
  modifiedMs: number
  isCloud: boolean
}

export interface LargeScanResult {
  sessionId: number
  drive: string
  files: LargeFile[]
  totalBytes: number
  truncated: boolean
  skippedDirs: number
  done: boolean
}

export interface LargeProgress {
  sessionId: number
  files: number
  bytes: number
}

export interface LargeDeleteOutcome {
  mode: "recovered" | "permanent"
  freedBytes: number
  recoveryBytes: number
}

export interface DupeFile {
  path: string
  size: number
  modifiedMs: number
  isCloud: boolean
}

export interface DupeGroup {
  hash: string
  size: number
  files: DupeFile[]
  keepIndex: number
}

export interface DupeScanResult {
  sessionId: number
  drive: string
  groups: DupeGroup[]
  walkedFiles: number
  hashedFiles: number
  truncated: boolean
  done: boolean
}

export interface DupeProgress {
  sessionId: number
  phase: "walk" | "prescreen" | "hash"
  done: number
  total: number
  groups: number
}

// ---------- M3：应用卸载 ----------

export interface InstalledApp {
  id: string
  name: string
  version: string
  publisher: string
  installDate: string
  /** 字节；未知为 0 */
  estimatedSize: number
  uninstallString: string
  installLocation: string
  isUwp: boolean
}

export interface ResidueDir {
  path: string
  size: number
  /** 名称匹配 / 发布者匹配 / 注册表安装位置 */
  reason: string
}

export interface ResidueInput {
  path: string
  size: number
}

// ---------- M3：空间分析 ----------

export interface DirNode {
  name: string
  path: string
  size: number
  fileCount: number
  /** 主导文件类型类别 id（0-7，见 SPACE_CATEGORY_LABELS） */
  dominant: number
  hasChildren: boolean
  children: DirNode[]
}

export interface SpaceScanResult {
  sessionId: number
  drive: string
  done: boolean
  totalBytes: number
  fileCount: number
  dirCount: number
  skippedDirs: number
  root: DirNode
}

export interface DirDetail {
  path: string
  name: string
  size: number
  fileCount: number
  dominant: number
  children: DirNode[]
}

export interface SpaceProgress {
  sessionId: number
  dirs: number
  files: number
  bytes: number
}

/** 与 Rust space::CATEGORY_LABELS 一致 */
export const SPACE_CATEGORY_LABELS = [
  "其他",
  "影音",
  "图片",
  "音频",
  "压缩包",
  "镜像",
  "安装包",
  "文档",
] as const

/** 大文件删除的 5GB 恢复区上限（与 Rust clean::RECOVERY_MAX_FILE_BYTES 一致） */
export const RECOVERY_MAX_FILE_BYTES = 5 * 1024 * 1024 * 1024

export function fileType(path: string): { label: string; cls: string } {
  const name = path.split("\\").pop() || ""
  const dot = name.lastIndexOf(".")
  // 无扩展名或扩展名异常长（哈希名等）→ 归为"文件"
  if (dot <= 0 || dot === name.length - 1 || name.length - dot - 1 > 5) {
    return { label: "文件", cls: "t-other" }
  }
  const ext = name.slice(dot + 1).toLowerCase()
  if (["mp4", "mkv", "avi", "mov", "wmv", "flv", "m4v", "webm", "ts"].includes(ext))
    return { label: "影音", cls: "t-video" }
  if (["jpg", "jpeg", "png", "gif", "bmp", "webp", "raw", "psd", "heic"].includes(ext))
    return { label: "图片", cls: "t-image" }
  if (["mp3", "wav", "flac", "ape", "ogg", "m4a", "aac"].includes(ext))
    return { label: "音频", cls: "t-audio" }
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].includes(ext))
    return { label: "压缩包", cls: "t-archive" }
  if (["iso", "img", "vhd", "vhdx", "gho", "wim", "dmg"].includes(ext))
    return { label: "镜像", cls: "t-image-disk" }
  if (["exe", "msi", "msix", "appx"].includes(ext))
    return { label: "安装包", cls: "t-installer" }
  if (["doc", "docx", "xls", "xlsx", "ppt", "pptx", "pdf", "txt", "md"].includes(ext))
    return { label: "文档", cls: "t-doc" }
  return { label: ext.toUpperCase(), cls: "t-other" }
}

export const CATEGORY_ICONS: Record<string, string> = {
  temp: "🗂️",
  recycle: "🗑️",
  browser: "🌐",
  update: "⟳",
  logs: "📄",
  thumbnail: "🖼️",
}

export function fmtSize(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0 B"
  if (n < 1024) return `${n} B`
  const units = ["KB", "MB", "GB", "TB"]
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`
}

export function fmtTime(ms: number): string {
  const d = new Date(ms)
  const p = (x: number) => String(x).padStart(2, "0")
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

export function daysLeft(expiresAtMs: number): number {
  return Math.max(0, Math.ceil((expiresAtMs - Date.now()) / 86_400_000))
}
