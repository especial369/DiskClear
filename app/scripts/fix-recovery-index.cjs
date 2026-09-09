// 一次性修复：清除恢复区索引中的悬空测试条目（recovery_path 指向已不存在的文件）
const fs = require("fs")
const path = "C:\\$DiskClear\\Recovery\\index.json"
const idx = JSON.parse(fs.readFileSync(path, "utf8"))
const before = idx.entries.length
idx.entries = idx.entries.filter((e) => fs.existsSync(e.recoveryPath))
fs.writeFileSync(path, JSON.stringify(idx, null, 2))
console.log(`entries: ${before} -> ${idx.entries.length}`)
