<script setup lang="ts">
defineProps<{
  open: boolean
  title: string
  danger?: boolean
}>()

const emit = defineEmits<{
  close: []
}>()
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="mask" @click.self="emit('close')">
      <div class="dialog card" :class="{ danger }">
        <div class="title">{{ title }}</div>
        <div class="body">
          <slot />
        </div>
        <div class="footer">
          <slot name="footer" />
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.mask {
  position: fixed;
  inset: 0;
  background: rgba(15, 23, 42, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}

.dialog {
  width: 460px;
  max-width: 90vw;
  padding: 20px 22px;
  animation: pop 0.16s ease;
}

.dialog.danger .title {
  color: var(--danger);
}

.title {
  font-size: 16px;
  font-weight: 700;
  margin-bottom: 12px;
}

.body {
  font-size: 13px;
  color: var(--text-2);
  line-height: 1.7;
  max-height: 50vh;
  overflow-y: auto;
}

.footer {
  margin-top: 18px;
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}

@keyframes pop {
  from {
    opacity: 0;
    transform: scale(0.96);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}
</style>
