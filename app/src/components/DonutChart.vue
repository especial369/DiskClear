<script setup lang="ts">
import { computed } from "vue"

const props = withDefaults(
  defineProps<{
    percent: number
    size?: number
    stroke?: number
    label?: string
    color?: string
  }>(),
  { size: 132, stroke: 12, label: "", color: "#2563EB" },
)

const r = computed(() => (props.size - props.stroke) / 2)
const c = computed(() => 2 * Math.PI * r.value)
const dash = computed(() => `${(Math.min(100, Math.max(0, props.percent)) / 100) * c.value} ${c.value}`)
</script>

<template>
  <div class="donut" :style="{ width: size + 'px', height: size + 'px' }">
    <svg :width="size" :height="size">
      <circle
        :cx="size / 2"
        :cy="size / 2"
        :r="r"
        fill="none"
        stroke="var(--border)"
        :stroke-width="stroke"
      />
      <circle
        :cx="size / 2"
        :cy="size / 2"
        :r="r"
        fill="none"
        :stroke="color"
        :stroke-width="stroke"
        stroke-linecap="round"
        :stroke-dasharray="dash"
        :transform="`rotate(-90 ${size / 2} ${size / 2})`"
      />
    </svg>
    <div class="center">
      <div class="pct">{{ Math.round(percent) }}<span class="pct-sign">%</span></div>
      <div v-if="label" class="label">{{ label }}</div>
    </div>
  </div>
</template>

<style scoped>
.donut {
  position: relative;
  display: inline-flex;
}

.center {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}

.pct {
  font-size: 28px;
  font-weight: 700;
  line-height: 1;
}

.pct-sign {
  font-size: 15px;
  font-weight: 600;
  margin-left: 1px;
}

.label {
  margin-top: 4px;
  font-size: 11px;
  color: var(--text-2);
}
</style>
