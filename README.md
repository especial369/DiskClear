# DiskClear 盘清

> 轻量级 Windows 盘符清理工具 —— 快速识别并安全清理磁盘中的垃圾文件、大文件与重复文件，释放存储空间。

DiskClear（盘清）是一款面向 Windows 10 / 11 用户的本地磁盘清理工具，采用 **Tauri 2 + Vue 3 + Rust** 构建。所有扫描与清理均在本地完成，**零上传、零常驻后台**，删除操作默认先移入应用恢复区，可随时撤销，做到"零误删可恢复"。

## ✨ 功能特性

| 模块 | 说明 |
|------|------|
| 🧹 一键清理 | 扫描系统临时文件、回收站、浏览器缓存、Windows 更新缓存、系统日志、缩略图缓存等常见垃圾，分类展示可释放空间，勾选后一键清理 |
| 📦 大文件查找 | 按阈值（默认 100MB）扫描大文件，降序排列；支持打开所在目录、加入忽略列表；超限文件走红色警示 + 二次确认 |
| 🔁 重复文件检测 | 按文件大小分组 → 首 4KB 预筛 → 全量 MD5 校验精确识别重复文件；推荐保留最新 / 路径最短项，一键清理其余 |
| ♻️ 恢复区 | 所有删除默认移入 `<盘>:\$DiskClear\Recovery\`，按批次展示并可恢复 / 手动清空；7 天自动过期（可配置） |
| ⚙️ 设置中心 | 大文件阈值、扫描线程数、恢复区保留天数、白名单目录、FAT32 超大文件策略、主题等 |

**安全设计**：系统关键目录白名单保护（`C:\Windows`、`Program Files` 等）→ 文件级例外清单放行明确缓存类型；文件占用自动跳过不中断；OneDrive 占位文件默认不清理；目录遍历不跟随 Junction，防止循环。

## 🛠 技术栈

| 层级 | 技术 |
|------|------|
| 桌面框架 | [Tauri 2.x](https://tauri.app/)（Rust + Web，安装包 ~10MB） |
| 前端 | Vue 3（`<script setup>`）+ TypeScript + Vite + lucide-vue-next 图标 |
| 后端 | Rust（多线程扫描 / MD5 哈希 / Windows API 交互） |
| 核心依赖 | `tauri`、`tauri-plugin-opener`、`serde`、`md5`、`windows-sys` |

## 📁 目录结构

```
DiskClear/
├── app/                        # 主应用（前端 + Tauri 壳）
│   ├── src/                    # Vue 3 前端源码
│   │   ├── pages/              # 页面：QuickClean / LargeFiles / Duplicates / Recovery / SettingsPage
│   │   ├── components/         # 通用组件：SideBar / StatusBar / DonutChart / AppModal 等
│   │   ├── api.ts              # Tauri IPC 调用封装
│   │   └── store.ts            # 全局状态
│   ├── src-tauri/              # Rust 后端
│   │   ├── src/                # scan / clean / recovery / dupes / bigfiles / settings / walker ...
│   │   └── tauri.conf.json     # Tauri 应用配置
│   ├── scripts/                # 辅助脚本（如恢复区索引修复）
│   └── package.json
├── asset/                      # 设计资源（应用图标、界面设计稿）
├── doc/                        # 产品文档（PRD）
└── README.md
```

## 🚀 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) ≥ 20.19（Vite 8 要求）
- [Rust](https://www.rust-lang.org/tools/install)（stable，含 Cargo）
- Windows 10 / 11（64 位），需 [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) 运行时

### 开发调试

```bash
cd app
npm install
npm run tauri dev        # 启动带热更新的桌面应用开发环境
```

### 构建与打包

```bash
npm run build            # 仅构建前端（vue-tsc 类型检查 + vite build）
npm run tauri build      # 打包桌面安装包（.msi / .exe）
```

## 🧩 后端模块一览

| 文件 | 职责 |
|------|------|
| `scan.rs` | 盘符扫描与会话管理（可取消） |
| `clean.rs` | 分类清理、净释放统计 |
| `recovery.rs` | 恢复区索引、恢复 / 清除 / 自愈 |
| `dupes.rs` | 重复文件扫描（大小分组 + 4KB 预筛 + MD5） |
| `bigfiles.rs` | 大文件扫描与删除策略 |
| `settings.rs` | 设置读写、忽略列表 |
| `walker.rs` | 目录遍历（不跟随 Junction / 跳过无权限目录） |
| `cache.rs` / `category.rs` | 哈希缓存 / 清理类别定义 |

## 📚 相关文档

- [产品需求文档（PRD）v1.1](doc/DiskClear%20盘清%20-%20PRD产品需求文档%20v1.1.md) —— 完整功能规格、安全机制（恢复区 / 白名单 / 占用处理 / 边界场景）与里程碑规划

## 📄 许可证

本项目为私有项目，保留所有权利。
