<script setup lang="ts">
// 矩形树图（PRD 2.5）：自研 squarified 布局（Bruls et al.），SVG 渲染，无第三方依赖
// - 矩形面积 = 目录大小，颜色 = 主导文件类型
// - 点击下钻；右键上抛上下文事件；悬停原生 tooltip
import { computed } from "vue"
import { fmtSize, SPACE_CATEGORY_LABELS, type DirNode } from "../types"

const props = defineProps<{ nodes: DirNode[] }>()

const emit = defineEmits<{
  drill: [node: DirNode]
  context: [payload: { node: DirNode; x: number; y: number }]
}>()

/** 类别 0-7 配色（与 SPACE_CATEGORY_LABELS 对齐） */
const COLORS = [
  "#94a3b8", // 其他
  "#a855f7", // 影音
  "#ec4899", // 图片
  "#f59e0b", // 音频
  "#10b981", // 压缩包
  "#06b6d4", // 镜像
  "#3b82f6", // 安装包
  "#22c55e", // 文档
]

const W = 1000
const H = 560

interface Rect {
  node: DirNode
  x: number
  y: number
  w: number
  h: number
}

/** 行的最差纵横比（Bruls et al. 2000） */
function worst(areas: number[], length: number): number {
  const s = areas.reduce((a, b) => a + b, 0)
  if (s <= 0 || areas.length === 0) return Infinity
  const max = Math.max(...areas)
  const min = Math.min(...areas)
  const l2 = length * length
  const s2 = s * s
  return Math.max((l2 * max) / s2, s2 / (l2 * min))
}

function squarify(
  entries: { node: DirNode; area: number }[],
  x: number,
  y: number,
  w: number,
  h: number,
  out: Rect[],
) {
  if (entries.length === 0 || w < 1 || h < 1) return
  if (entries.length === 1) {
    out.push({ node: entries[0].node, x, y, w, h })
    return
  }
  const total = entries.reduce((s, e) => s + e.area, 0)
  if (total <= 0) return
  // 面积归一化到当前矩形
  const scaled = entries.map((e) => ({ node: e.node, area: (e.area / total) * w * h }))
  const shortSide = Math.min(w, h)
  let row: { node: DirNode; area: number }[] = []
  let rest = scaled
  while (rest.length > 0) {
    const candidate = [...row, rest[0]]
    if (row.length === 0) {
      row = candidate
      rest = rest.slice(1)
      continue
    }
    const improved =
      worst(candidate.map((c) => c.area), shortSide) <= worst(row.map((c) => c.area), shortSide)
    if (improved) {
      row = candidate
      rest = rest.slice(1)
    } else {
      break
    }
  }
  const rowSum = row.reduce((s, r) => s + r.area, 0)
  if (w >= h) {
    // 竖条贴左
    const stripW = rowSum / h
    let yy = y
    for (const r of row) {
      const ih = r.area / stripW
      out.push({ node: r.node, x, y: yy, w: stripW, h: ih })
      yy += ih
    }
    squarify(rest, x + stripW, y, w - stripW, h, out)
  } else {
    // 横条贴上
    const stripH = rowSum / w
    let xx = x
    for (const r of row) {
      const iw = r.area / stripH
      out.push({ node: r.node, x: xx, y, w: iw, h: stripH })
      xx += iw
    }
    squarify(rest, x, y + stripH, w, h - stripH, out)
  }
}

const rects = computed<Rect[]>(() => {
  const items = props.nodes
    .filter((n) => n.size > 0)
    .map((n) => ({ node: n, area: n.size }))
  const out: Rect[] = []
  squarify(items, 0, 0, W, H, out)
  return out
})

function colorOf(node: DirNode): string {
  return COLORS[node.dominant] ?? COLORS[0]
}

/** 矩形是否放得下两行标签 */
function fitLabel(r: Rect): boolean {
  return r.w > 86 && r.h > 44
}

function fitSizeOnly(r: Rect): boolean {
  return r.w > 56 && r.h > 24
}

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1) + "…" : s
}
</script>

<template>
  <div class="treemap-wrap">
    <svg :viewBox="`0 0 ${W} ${H}`" class="treemap">
      <g v-for="r in rects" :key="r.node.path">
        <rect
          :x="r.x + 1"
          :y="r.y + 1"
          :width="Math.max(0, r.w - 2)"
          :height="Math.max(0, r.h - 2)"
          rx="7"
          class="tm-rect"
          :fill="colorOf(r.node)"
          @click="emit('drill', r.node)"
          @contextmenu.prevent="emit('context', { node: r.node, x: $event.clientX, y: $event.clientY })"
        >
          <title>{{ r.node.name }}&#10;{{ fmtSize(r.node.size) }} · {{ r.node.fileCount }} 个文件</title>
        </rect>
        <text
          v-if="fitLabel(r)"
          :x="r.x + 12"
          :y="r.y + 24"
          class="tm-name"
          @click="emit('drill', r.node)"
          @contextmenu.prevent="emit('context', { node: r.node, x: $event.clientX, y: $event.clientY })"
        >
          {{ truncate(r.node.name, Math.floor(r.w / 9)) }}
        </text>
        <text
          v-if="fitLabel(r)"
          :x="r.x + 12"
          :y="r.y + 42"
          class="tm-size"
          @click="emit('drill', r.node)"
        >
          {{ fmtSize(r.node.size) }} · {{ r.node.fileCount }} 文件
        </text>
        <text
          v-else-if="fitSizeOnly(r)"
          :x="r.x + 8"
          :y="r.y + 17"
          class="tm-name"
          @click="emit('drill', r.node)"
        >
          {{ truncate(r.node.name, Math.floor(r.w / 8)) }}
        </text>
      </g>
      <text v-if="rects.length === 0" :x="W / 2" :y="H / 2" class="tm-empty" text-anchor="middle">
        当前目录无子目录
      </text>
    </svg>
    <div class="legend">
      <span v-for="(label, i) in SPACE_CATEGORY_LABELS" :key="label" class="legend-item">
        <span class="dot" :style="{ background: COLORS[i] }"></span>{{ label }}
      </span>
    </div>
  </div>
</template>

<style scoped>
.treemap-wrap {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.treemap {
  width: 100%;
  height: auto;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 12px;
}

.tm-rect {
  cursor: pointer;
  transition: opacity 0.12s;
}

.tm-rect:hover {
  opacity: 0.82;
}

.tm-name {
  font-size: 14px;
  font-weight: 600;
  fill: #fff;
  pointer-events: none;
  paint-order: stroke;
  stroke: rgba(0, 0, 0, 0.25);
  stroke-width: 2px;
}

.tm-size {
  font-size: 12px;
  fill: rgba(255, 255, 255, 0.88);
  pointer-events: none;
  paint-order: stroke;
  stroke: rgba(0, 0, 0, 0.25);
  stroke-width: 2px;
}

.tm-empty {
  font-size: 15px;
  fill: var(--muted);
}

.legend {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  padding: 0 4px;
}

.legend-item {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-2);
}

.dot {
  width: 10px;
  height: 10px;
  border-radius: 3px;
}
</style>
