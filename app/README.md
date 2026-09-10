# app — DiskClear 盘清（前端 + Tauri 壳）

本目录为 DiskClear 盘清桌面的主应用源码，包含 Vue 3 前端（`src/`）与 Rust 后端（`src-tauri/`）。

- 项目总览、开发与构建说明见仓库根目录 [README](../README.md)
- 完整功能规格见 [PRD v1.1](../doc/DiskClear%20盘清%20-%20PRD产品需求文档%20v1.1.md)

## 常用命令

```bash
npm install        # 安装依赖
npm run tauri dev  # 启动桌面开发环境（带热更新）
npm run build      # 类型检查 + 构建前端
npm run tauri build  # 打包安装包
```
