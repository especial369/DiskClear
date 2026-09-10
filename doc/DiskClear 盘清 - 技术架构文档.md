# DiskClear 盘清 — 技术架构文档

> 版本：v1.0 · 对应代码版本：0.1.0  
> 配套文档：[产品需求文档（PRD）v1.1](./DiskClear%20盘清%20-%20PRD产品需求文档%20v1.1.md)

---

## 1. 文档目的

本文面向开发者与维护者，系统阐述 DiskClear 盘清的整体架构、模块职责、跨进程数据流、关键算法与安全机制，作为阅读与演进代码的地图。所有结论均来自对当前源码（`app/` 前端 + `app/src-tauri/` 后端）的实际解析，不依赖 PRD 的规划性描述。

---

## 2. 系统总体架构

DiskClear 是一款 **Tauri 2 桌面应用**：Rust 进程承载核心逻辑与文件系统操作，内嵌的 WebView2 渲染 Vue 3 UI；二者通过 Tauri 的 IPC（`invoke` 命令 + `emit` 事件）通信。

### 2.1 分层视图

```
┌──────────────────────────────────────────────────────────┐
│                   UI Layer  (Vue 3 + TS)                  │
│  App.vue / SideBar / StatusBar / pages(QuickClean...)     │
│  api.ts(IPC 封装)  store.ts(状态)  types.ts(类型契约)      │
└───────────────────────────┬──────────────────────────────┘
                            │  invoke(命令) ↑↓  listen(事件) ↓
┌───────────────────────────┴──────────────────────────────┐
│                    IPC Bridge  (Tauri)                   │
│        capabilities/default.json 权限白名单               │
└───────────────────────────┬──────────────────────────────┘
                            │
┌───────────────────────────┴──────────────────────────────┐
│                Rust Core  (src-tauri/src)                 │
│  commands.rs(命令层)  lib.rs(AppState+入口)               │
│  ┌─────────┬─────────┬──────────┬───────────┬──────────┐ │
│  │ Scanner │ Cleaner │ Recovery │ Dupes/Big │ Settings  │ │
│  │ scan.rs │clean.rs │recovery  │dupes/big  │settings   │ │
│  └────┬────┴────┬────┴────┬─────┴─────┬─────┴─────┬────┘ │
│       │  walker.rs(并行遍历) │   cache.rs │   category.rs│ │
│       └──────────┬─────────┴────┬──────┴──────┬─────┘    │
│              util.rs(Win32 API / 长路径 / 进程检测)        │
└───────────────────────────┬──────────────────────────────┘
                            │
┌───────────────────────────┴──────────────────────────────┐
│                  Storage / File System                    │
│  %APPDATA%\com.diskclear.app\ {settings,dupe_cache,stats} │
│  <盘>:\$DiskClear\Recovery\<batch>\  (每盘恢复区+index)   │
└──────────────────────────────────────────────────────────┘
```

### 2.2 技术选型

| 层级 | 技术 | 说明 |
|------|------|------|
| 桌面框架 | Tauri 2.x | Rust 壳 + WebView2 渲染，安装包 ~10MB |
| 前端 | Vue 3 (`<script setup>`) + TypeScript + Vite 8 | 无路由库，单页内 `store.page` 切换 |
| 图标 | lucide-vue-next | 与 PRD「Lucide 风格」一致 |
| 后端语言 | Rust (edition 2021) | 多线程扫描 / 哈希 |
| 核心依赖 | `serde`/`serde_json`、`md5`、`windows-sys`、`tauri-plugin-opener` | 无 chrono/dirs 等额外依赖，时间戳与目录定位自行实现 |

---

## 3. 工程结构

```
DiskClear/
├── app/                         # 主应用
│   ├── src/                     # Vue 前端
│   │   ├── pages/               # 5 个已实现页面 + 2 个占位
│   │   ├── components/          # SideBar/StatusBar/DonutChart/AppModal/Placeholder
│   │   ├── api.ts               # invoke 封装（24 个命令的 TS 入口）
│   │   ├── store.ts             # reactive 全局状态 + 主题
│   │   ├── types.ts             # 与 Rust serde camelCase 对齐的接口 + 工具函数
│   │   └── main.ts              # createApp 入口
│   ├── src-tauri/
│   │   ├── src/                 # Rust 源码（见 §5）
│   │   ├── capabilities/default.json   # 主窗口权限：core:default + opener:default
│   │   ├── tauri.conf.json      # 窗口 1280×820 / 前端 dist 路径
│   │   ├── Cargo.toml           # release: lto + strip + panic=abort
│   │   └── build.rs             # tauri_build::build()
│   ├── scripts/fix-recovery-index.cjs   # 恢复区索引修复脚本
│   └── package.json
├── asset/                       # 应用图标与界面设计稿
└── doc/                         # PRD 与本文档
```

---

## 4. 应用启动流程

### 4.1 后端启动（`lib.rs::run`）

```
main.rs  ──>  app_lib::run()
   │
   ├─ tauri::Builder::default()
   ├─ .plugin(tauri_plugin_opener::init())          # 注册 opener 插件
   ├─ .manage(AppState::new())                       # 注入全局状态
   ├─ .setup(|app| {
   │      std::thread::spawn(|| recovery::startup_maintenance(&state))
   │   })                                           # 后台线程：恢复区过期清理 + 自愈，不阻塞启动
   ├─ .invoke_handler(generate_handler![ ... 24 命令 ... ])
   └─ .run(generate_context!())
```

### 4.2 前端启动（`main.ts` → `App.vue`）

```
main.ts  ──>  createApp(App).mount("#app")
   │
   └─ App.vue onMounted:
        ├─ api.getSettings() → applyTheme(theme)    # 应用主题
        └─ store.refreshStats()                     # 拉取状态栏统计
```

`main.rs` 顶部 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` 在 release 下隐藏控制台窗口。

---

## 5. 后端架构详解

### 5.1 全局状态 `AppState`（`lib.rs`）

所有跨命令、跨线程的共享状态集中于 `AppState`，字段全部 `Arc` 化以便命令层 clone 后传入 `spawn_blocking`：

| 字段 | 类型 | 职责 |
|------|------|------|
| `sessions` | `Arc<Mutex<HashMap<u32, ScanSession>>>` | 一键清理扫描会话（最多保留 3 个，溢出淘汰最旧） |
| `big_sessions` | `…<LargeScanSession>` | 大文件扫描会话 |
| `dupe_sessions` | `…<DupeScanSession>` | 重复文件扫描会话 |
| `next_session` | `Arc<AtomicU32>` | 自增 id，**扫描会话与清理批次号共用**此计数器 |
| `cancels` | `…<HashMap<u32, Arc<AtomicBool>>>` | 各会话的取消标志 |
| `recovery_lock` | `Arc<Mutex<()>>` | 恢复区写锁，串行化恢复区索引读写 |
| `hash_cache` | `Arc<HashCache>` | 重复文件哈希缓存 |

### 5.2 命令层（`commands.rs`）

`commands.rs` 是前端 `invoke` 的全部入口，共注册 **24 个命令**（见 `lib.rs` 的 `generate_handler!`）。它仅做参数解包与线程调度，业务逻辑下沉到各模块：

- **同步快命令**（直接返回）：`list_drives`、`get_env_info`、`get_settings`、`get_stats`、`list_recovery`、`open_path` 等。
- **长任务命令**：立即返回 `session_id`，后台线程执行并通过事件推进度（见 §6）。
- **阻塞 IO 命令**：`clean`、`delete_large_file`、`delete_dupe_files` 用 `spawn_blocking` 包裹，避免阻塞 Tauri 异步运行时。

### 5.3 扫描器 `scan.rs`（一键清理）

负责 PRD 2.2 的垃圾文件扫描，**按分类串行遍历**（M1 范围内分类均为缓存目录，量级可控）：

- `start_scan`：分配 session id（淘汰至 ≤3 个），后台线程跑 `run_scan`。
- `run_scan`：先探测环境（`is_admin`/`running_browsers`/`wuauserv_running`），逐分类遍历，每完成一个分类 `emit("scan-progress")`，全部完成 `emit("scan-done")`。
- `walk`：**迭代式栈遍历**（防深目录递归栈溢出）；不跟随 reparse point；跳过云占位；按 `category::matches_name` 过滤文件名；每 50ms `yield_now` 让出 CPU。
- `scan_recycle`：遍历 `<drive>:\$Recycle.Bin\<SID>\$I*` 元数据，解析原始路径与大小，映射到 `$R*` 实体。
- `parse_recycle_meta`：解析回收站 `$I` 二进制格式（v1 定长 520B / v2+ 变长），含单元测试覆盖三个版本与垃圾输入。

### 5.4 清理器 `clean.rs`

实现 PRD 2.2 交互流程 + 6.1 恢复区 + 6.4 占用跳过。文档头注释明确「**纵深防御四道防线**」：

1. 扫描阶段：文件只能来自注册分类根目录；
2. 清理阶段：删除前 `under_any_root` 再次校验路径归属（词法前缀，已规避 `C:\Windows\Temp` 误匹配 `C:\Windows\TempXYZ`）；
3. 删除方式：一律 `rename` 到同盘恢复区，不直接 `unlink`；
4. 占用失败：重试 1 次（间隔 80ms）后计入 `skippedLocked`，绝不中断。

关键函数：

| 函数 | 职责 |
|------|------|
| `run_clean` | 一键清理：从会话取所选分类文件 → 逐个 `move_one`，期间 `emit("clean-progress")`，完成后 `add_freed_total` 累计统计 |
| `move_one` | 单文件移入恢复区的共享实现：确定恢复区根 → 建批次目录 → rename（重试）→ 回收站额外删 `$I` 元数据 → 追加 `RecoveryEntry` |
| `move_paths_to_recovery` | 通用入口，重复文件清理复用；**云占位文件一律拒绝**（避免波及云端） |
| `delete_large_file` | PRD 2.3 分级策略：≤5GB 移恢复区；>5GB 仅当 `allow_permanent=true` 时永久删除 |

> 注：时间戳 `chrono_stamp` 用 Howard Hinnant 算法从 UNIX 天数反推 (年,月,日)，**无 chrono 依赖**；批次号格式 `YYYYMMDD_HHMMSS_<seq>`。

### 5.5 恢复区 `recovery.rs`（PRD 6.1，v1.1 按盘分布）

**位置策略**：被清理文件移入其**所在盘**根下 `\$DiskClear\Recovery\<batch>\`（同盘 rename 零拷贝）；C 盘无盘根写权限时回退 `%LOCALAPPDATA%\DiskClear\Recovery\`。恢复区目录设 `HIDDEN | SYSTEM` 属性。

**索引模型**：每个恢复区根一份 `index.json`（`RecoveryIndex.entries: Vec<RecoveryEntry>`），写入用 `tmp + rename` 原子替换。

**三步维护**（`maintenance_for_root`，可测试）：
1. **过期清除**：`expires_at_ms <= now` 的条目物理删除文件 + 移除索引；
2. **孤儿自愈**：磁盘上存在但索引未引用的批次 → 重建条目（原始路径记为"未知"，避免误伤正常批次——`known` 集合用规范化批次目录路径比较）；
3. **空批次清理**：删除空批次目录。

含 4 个回归测试：恢复闭环、永久删除闭环、孤儿自愈、**维护不得重复登记正常批次**（针对历史 bug 的幂等性回归）、过期清除。

### 5.6 重复文件检测 `dupes.rs`（PRD 2.4）

**三级算法**：

```
阶段1 walk   : walker::walk_parallel（≥1MB 文件，云占位排除）→ 按大小分组
阶段2 prescreen: 同大小组内读首 4KB → FNV-1a 预筛 → 仅保留 ≥2 副本的子组
阶段3 hash   : 全量 MD5（1MB 分块）→ 命中缓存(size+mtime 未变)则复用 → ≥2 副本成组
```

- **推荐保留**（`pick_keep`）：修改时间最新优先，相同则路径最短；
- **进度推送**：`PhaseProgress` 每 250ms `emit("dupe-progress")`，阶段名 `walk`/`prescreen`/`hash`；完成 `emit("dupe-done")`；
- **上限保护**：`MAX_HASHED_FILES = 50_000`，超出置 `truncated`；
- **结果排序**：按可释放空间（`wasted = size × (副本数−1)`）降序。

### 5.7 大文件查找 `bigfiles.rs`（PRD 2.3）

复用 `walker::walk_parallel` 全盘扫描超过阈值的文件：

- 独立**进度采样线程**每 300ms `emit("large-progress")`（文件数/字节数），与遍历解耦；
- `collect` 收集至 `MAX_FILES = 20_000` 上限；
- 忽略列表（`settings.ignored_files`）过滤 + 大小降序；
- 完成 `emit("large-done")`。

### 5.8 并行遍历器 `walker.rs`（核心基础设施）

供 `dupes` 与 `bigfiles` 复用，是后端性能关键。

**工作队列模型**（无竞态）：
- `queue`：共享 `VecDeque<PathBuf>`，多线程取目录；
- `pending`：`AtomicUsize`，= 已发现未完成目录数；
- 终止条件：worker 取到 `None`（队列空）**且** `pending == 0` → 退出；否则短暂 `sleep(2ms)` 等待新目录入队。

**边界处理**：
- 目录是符号链接或 reparse point → **不跟随**（防 junction 循环，含专门测试 `walk_parallel_finds_files_and_skips_junction_loops` 用 `mklink /J` 造环验证）；
- 文件符号链接跳过；
- 阈值过滤在 worker 内完成，避免全盘小文件进入内存；
- worker 本地缓冲 128 条后批量 `send`，减少锁竞争。

**哈希工具**：`fnv64a`（首 4KB 预筛，非加密，速度优先，含标准测试向量）、`md5_of_file`（1MB 分块）、`read_head`（读首 N 字节）。

### 5.9 设置 `settings.rs` / 缓存 `cache.rs` / 类别 `category.rs` / 工具 `util.rs`

- **`settings.rs`**：`Settings` 结构（保留天数/白名单目录/FAT32 策略/大文件阈值/扫描线程数/主题/忽略列表），存 `%APPDATA%\com.diskclear.app\settings.json`，`#[serde(default)]` 保证向前兼容。
- **`cache.rs`**：`HashCache` 惰性加载 `dupe_cache.json`，命中条件 = size + mtime 均未变；超 10 万条整体重置；`save_if_dirty` 脏标记持久化。
- **`category.rs`**：6 个清理分类定义（temp/recycle/browser/update/logs/thumbnail）；`roots_for` 解析各分类扫描根（浏览器缓存按 Profile 枚举 Chrome/Edge/Firefox）；`NameMatch::CacheDb` 限定缩略图类只清 `thumbcache_*/iconcache_*.db`——**分类根即 PRD 6.2 的「例外清单」**。
- **`util.rs`**：Win32 互操作与系统检测的核心：
  - `long_path`：加 `\\?\` 前缀支持 >260 字符长路径（UNC 转 `\\?\UNC\`）；
  - `list_drives`：`GetLogicalDrives` + `GetDiskFreeSpaceExW` + `GetVolumeInformationW`，跳过网络盘/光驱；
  - `is_reparse_dir` / `is_cloud_placeholder`：文件属性位检测；
  - `is_admin`（`net session`）/ `running_browsers`（tasklist）/ `wuauserv_running`（sc query）；
  - `move_path`：同盘 rename 优先，跨设备复制+删除兜底；
  - `set_hidden_system`：恢复区目录属性。

---

## 6. 前端架构详解

### 6.1 入口与状态（`main.ts` / `App.vue` / `store.ts`）

- **无路由库**：`App.vue` 用 `v-if` 按 `store.page` 切换页面（`clean`/`large`/`dupes`/`recovery`/`settings` + `space`/`uninstall` 占位）。
- **`store.ts`**：`reactive` 单例，持有当前页、状态栏统计（`freedTotalBytes`/`recoveryBytes`）、轻提示队列；`applyTheme` 根据 `prefers-color-scheme` 切换 `data-theme`。
- **`App.vue` onMounted**：拉取设置应用主题 + 刷新统计。

### 6.2 IPC 封装（`api.ts`）

`api` 对象把 24 个 Rust 命令封装为带类型的 TS 方法，参数名与 Rust `camelCase` 对齐。前端从不直接调 `invoke` 字符串，统一走 `api.*`。

### 6.3 类型契约（`types.ts`）

- 与 Rust `#[serde(rename_all = "camelCase")]` 一一对应的接口（`DriveInfo`/`CategoryResult`/`CleanResult`/`RecoveryEntry`/`Settings`/`LargeFile`/`DupeGroup` 等）；
- `RECOVERY_MAX_FILE_BYTES = 5GB` 与 Rust 常量保持一致，供前端判断是否需要走永久删除确认流程；
- 工具函数：`fmtSize`/`fmtTime`/`daysLeft`/`fileType`（按扩展名分类着色）。

### 6.4 页面与组件

| 页面 | 监听事件 | 主要 IPC 调用 | 说明 |
|------|----------|--------------|------|
| `QuickClean` | `scan-progress`/`scan-done`/`clean-progress` | `getEnvInfo`/`listDrives`/`startScan`/`cancelScan`/`getScanResult`/`clean` | 盘符环形图选择 → 扫描 → 勾选分类 → 确认清理 → 结果弹窗 |
| `LargeFiles` | `large-progress`/`large-done` | `getSettings`/`listDrives`/`startLargeScan`/`cancelLargeScan`/`getLargeScanResult`/`deleteLargeFile`/`addIgnoredFile`/`clearIgnoredFiles`/`openPath` | 阈值扫描、降序列表、>5GB 走永久删除确认 |
| `Duplicates` | `dupe-progress`/`dupe-done` | `listDrives`/`startDupeScan`/`cancelDupeScan`/`getDupeScanResult`/`deleteDupeFiles` | 三阶段进度、按组展示、推荐保留项标注 |
| `Recovery` | — | `listRecovery`/`restoreEntries`/`deleteRecovery`/`clearRecovery` | 按批次分组、单文件/批次恢复、清空 |
| `SettingsPage` | — | `getSettings`/`saveSettings`/`getEnvInfo` | 各设置项编辑保存 |

组件：`SideBar`（六项导航 + 安全提示）、`StatusBar`（累计释放/恢复区入口）、`DonutChart`（盘符占用环形图）、`AppModal`（通用弹窗，承载确认/进度/结果）、`Placeholder`（M3 未实现页占位）。

---

## 7. IPC 数据流

### 7.1 命令（请求-响应）

| 命令 | 模式 | 返回 |
|------|------|------|
| `list_drives` / `get_env_info` / `get_settings` / `get_stats` | 同步 | 数据快照 |
| `start_scan` / `start_large_scan` / `start_dupe_scan` | 异步启动 | `session_id`（立即返回） |
| `cancel_*` | 设置取消标志 | `void` |
| `get_*_result` | 查询会话 | 会话快照（`done`/`cancelled` 标志） |
| `clean` / `delete_large_file` / `delete_dupe_files` | `spawn_blocking` | `CleanResult`/`LargeDeleteOutcome` |
| `list_recovery` / `restore_entries` / `delete_recovery` / `clear_recovery` | 同步（持 `recovery_lock`） | 数量 |
| `save_settings` / `add_ignored_file` / `clear_ignored_files` | 同步 | `Settings` |
| `open_path` | opener 插件 | `void` |

### 7.2 事件（后端推送）

```
后端 emit                         前端 listen
─────────────────────────────────────────────
scan-progress / scan-done    ──>  QuickClean
clean-progress               ──>  QuickClean
large-progress / large-done  ──>  LargeFiles
dupe-progress / dupe-done     ──>  Duplicates
```

**长任务统一模式**：`start_xxx` 立即返回 `session_id` → 后台线程执行 → 期间周期性 `emit` 进度事件 → 完成时 `emit` `xxx-done` → 前端监听到 done 后调 `get_xxx_result(session_id)` 拉取完整结果。前端在 `onMounted` 注册 `unlisteners`，`onUnmounted` 统一清理，避免重复监听。

---

## 8. 核心数据结构

### 8.1 Rust 侧（节选）

```rust
// 扫描结果分类项
struct CategoryResult { id, name, desc, risk, file_count, total_size,
                        needs_admin, disabled, warning }

// 恢复区条目（持久化于各盘 index.json）
struct RecoveryEntry { id, batch_id, original_path, recovery_path, size,
                       deleted_at_ms, expires_at_ms, category_id, drive }

// 重复文件组
struct DupeGroup { hash, size, files: Vec<DupeFile>, keep_index }
```

### 8.2 持久化文件清单

| 路径 | 内容 | 维护方 |
|------|------|--------|
| `%APPDATA%\com.diskclear.app\settings.json` | 用户设置 | `settings.rs` |
| `%APPDATA%\com.diskclear.app\dupe_cache.json` | 哈希缓存（≤10万条） | `cache.rs` |
| `%APPDATA%\com.diskclear.app\stats.json` | 累计释放字节数 | `clean.rs` |
| `<盘>:\$DiskClear\Recovery\index.json` | 该盘恢复区索引 | `recovery.rs` |
| `<盘>:\$DiskClear\Recovery\<batch>\<seq>_<name>` | 被删文件实体 | `clean.rs` |
| `%LOCALAPPDATA%\DiskClear\Recovery\...` | C 盘回退恢复区 | `recovery.rs` |

---

## 9. 关键算法解析

### 9.1 并行遍历工作队列（`walker.rs`）

多线程共享一个待遍历目录队列，每个 worker：

```
loop {
  取一个目录 dir（无锁竞争 pop_front）
  ├─ Some(dir): read_dir，遇子目录（非 reparse）则 pending+1 并入队；
  │             遇文件（非符号链接）且 size≥threshold → 产出，pending−1
  └─ None: 若 pending==0 退出；否则 sleep(2ms) 重试
}
worker 结束前 flush 本地缓冲 + send Done
```

`pending` 计数法保证了「队列瞬时为空但仍有目录在处理」时不会误判终止——这是无竞态正确性的关键。

### 9.2 重复文件三级校验

大小分组（不同大小不可能重复）→ 首 4KB FNV-1a 预筛（大幅减少全量读盘 IO）→ 全量 MD5（1MB 分块）。预筛与全量哈希结果按 `(size, path)` 缓存，二次扫描时 `size + mtime` 未变即复用。三级校验使实际碰撞概率可忽略。

### 9.3 恢复区自愈与幂等性

`maintenance_for_root` 的 `known` 集合收集索引中各条目的**批次目录**（`recovery_path.parent()` 规范化），孤儿检测时用目录路径比较——历史上曾因比较对象不一致（存的是文件路径、比的是目录路径且带 `\\?\` 前缀）导致每次维护都把正常批次重复登记，现有专门回归测试 `maintenance_must_not_duplicate_known_batches` 守护。

---

## 10. 安全机制总览

DiskClear 的安全设计是纵深防御，贯穿扫描—清理—恢复全链路：

| 维度 | 机制 | 代码位置 |
|------|------|----------|
| 路径归属 | 删除前 `under_any_root` 二次校验，词法前缀防 `Temp`/`TempXYZ` 误匹配 | `clean.rs` |
| 白名单 | 分类根即例外清单；用户 `whitelist_dirs` 额外保护 | `category.rs` / `clean.rs` |
| 删除方式 | 一律 rename 到同盘恢复区，不直接 unlink；>5GB 永久删除需显式 `allow_permanent` | `clean.rs` |
| 云占位 | 重复文件清理一律拒绝云占位；大文件页才允许显式永久删除 | `clean.rs` |
| 链接循环 | 目录遍历不跟随 reparse point，文件符号链接跳过 | `walker.rs` / `scan.rs` |
| 长路径 | 所有 fs 操作经 `long_path` 加 `\\?\` 前缀 | `util.rs` |
| 占用容错 | 重试 1 次后计入 `skippedLocked`，不中断、不弹错 | `clean.rs` |
| 恢复区隔离 | 目录设 `HIDDEN\|SYSTEM`；原子写入 `index.json`；写锁串行化 | `recovery.rs` |
| 过期清理 | 启动 + 清理后自动维护，按保留天数物理删除 | `recovery.rs` |
| 自愈 | 索引缺失时从孤儿批次重建；目录被删时重建空恢复区 | `recovery.rs` |
| 权限感知 | 非管理员标注提示、`update` 分类禁用勾选 | `scan.rs` / `commands.rs` |

---

## 11. 配置与构建

### 11.1 Tauri 配置（`tauri.conf.json`）

- 窗口 1280×820，最小 980×640，居中；
- `beforeDevCommand: npm run dev`，`devUrl: localhost:1420`，`frontendDist: ../dist`；
- bundle targets = all，图标含 ico/icns/png 全平台。

### 11.2 权限（`capabilities/default.json`）

主窗口仅授予 `core:default` + `opener:default`，最小权限原则。

### 11.3 Vite 配置（`vite.config.ts`）

为 Tauri 定制：`clearScreen:false`、固定端口 1420（`strictPort`）、忽略 `src-tauri/**` 监听、支持 `TAURI_DEV_HOST` 远程调试 HMR。

### 11.4 Cargo release profile

`codegen-units=1` + `lto=true` + `opt-level=3` + `panic=abort` + `strip=true`，最小化安装包体积。

### 11.5 构建命令

```bash
cd app
npm install
npm run tauri dev     # 开发（热更新）
npm run build         # 前端类型检查 + 构建
npm run tauri build   # 打包 .msi/.exe
```

---

## 12. 测试覆盖

后端内嵌单元测试（`cargo test`），覆盖关键正确性与历史回归：

| 模块 | 测试 |
|------|------|
| `walker.rs` | FNV-1a / MD5 标准向量、junction 环不循环遍历 |
| `scan.rs` | 回收站 `$I` v1/v2 解析、垃圾输入 |
| `clean.rs` | `under_any_root` 前缀校验、`civil_from_days` 日期、大文件分级删除策略 |
| `recovery.rs` | 恢复闭环、永久删除闭环、孤儿自愈、**维护幂等性回归**、过期清除 |
| `dupes.rs` | `pick_keep` 优先级、`wasted` 计算 |
| `cache.rs` | 命中需 size+mtime 均未变 |
| `category.rs` | 缩略图只匹配 `*.db`、回收站根随盘符 |
| `util.rs` | 长路径前缀、盘符根提取 |
| `settings.rs` | 默认值往返序列化 |

---

## 13. 当前实现范围与演进

- **已实现（M1–M2）**：一键清理、大文件查找、重复文件检测、恢复区、设置中心。
- **占位（M3）**：磁盘空间分析（Treemap）、应用卸载——前端 `Placeholder` 组件提示，后端无对应模块。
- **PRD 规划 v2**：定时清理、开机自启、注册表清理——依赖常驻后台或高风险操作，已明确移出 v1。

演进时建议：新增清理分类须同步评审 `category.rs` 根目录与 PRD 6.2 白名单两侧；新增持久化结构沿用 `%APPDATA%\com.diskclear.app\` 约定并保持 `#[serde(default)]` 向前兼容。

---

*文档完。如需补充某一模块的逐行级解析或时序图，可在后续版本扩展。*
