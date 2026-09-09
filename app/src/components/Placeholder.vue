<script setup lang="ts">
defineProps<{
  milestone: string
  title: string
  desc: string
  icon: string
}>()
</script>

<template>
  <div class="placeholder card">
    <div class="icon">{{ icon }}</div>
    <div class="title">{{ title }}</div>
    <div class="badge">{{ milestone }}</div>
    <p class="desc">{{ desc }}</p>
  </div>
</template>

<style scoped>
.placeholder {
  max-width: 560px;
  margin: 12vh auto 0;
  padding: 44px 40px;
  text-align: center;
}

.icon {
  font-size: 44px;
}

.title {
  font-size: 20px;
  font-weight: 700;
  margin-top: 14px;
}

.badge {
  display: inline-block;
  margin-top: 10px;
  font-size: 12px;
  font-weight: 600;
  color: var(--primary);
  background: var(--primary-soft);
  border-radius: 99px;
  padding: 4px 14px;
}

.desc {
  margin-top: 16px;
  color: var(--text-2);
  font-size: 13px;
  line-height: 1.8;
}
</style>
