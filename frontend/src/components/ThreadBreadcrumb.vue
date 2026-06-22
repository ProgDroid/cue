<script setup lang="ts">
import { useCatalogueStore } from '@/stores/catalogue'

const store = useCatalogueStore()
</script>

<template>
  <div v-if="store.thread.length" class="thread-breadcrumb">
    <span class="thread-eyebrow">thread</span>
    <button class="thread-pill" @click="store.clearThread()">
      Library
    </button>
    <button
      v-for="(step, i) in store.thread"
      :key="i"
      :class="['thread-pill', { 'thread-pill--active': i === store.thread.length - 1 }]"
      @click="store.stepThread(i)"
    >
      {{ step.label }}
    </button>
  </div>
</template>

<style scoped>
.thread-breadcrumb {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 11px;
}

.thread-eyebrow {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10px;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--text-faint, #4f555f);
  margin-right: 2px;
}

.thread-pill {
  padding: 4px 11px;
  background: var(--surface-2, #1d2129);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 999px;
  color: var(--text-secondary, #c2c7d0);
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  white-space: nowrap;
  transition: border-color 120ms ease, color 120ms ease, background 120ms ease;
}

.thread-pill:hover {
  border-color: rgba(245, 197, 24, 0.4);
  color: var(--text-primary, #f4f5f7);
}

/* Last/active pill: amber-tinted */
.thread-pill--active {
  background: rgba(245, 197, 24, 0.12);
  border-color: rgba(245, 197, 24, 0.35);
  color: var(--accent, #f5c518);
}

.thread-pill--active:hover {
  background: rgba(245, 197, 24, 0.18);
  border-color: rgba(245, 197, 24, 0.5);
  color: var(--accent, #f5c518);
}
</style>
