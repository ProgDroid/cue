<script setup lang="ts">
import { useCatalogueStore } from '@/stores/catalogue'
import RefineChips from '@/components/RefineChips.vue'

const store = useCatalogueStore()
</script>

<template>
  <div class="answer-context">
    <div class="answer-left">
      <span class="answer-icon" aria-hidden="true">✦</span>
      <div class="answer-text">
        <div class="answer-line">{{ store.line }}</div>
        <div class="answer-sub">{{ store.sub }}</div>
      </div>
    </div>
    <RefineChips />
  </div>
</template>

<style scoped>
.answer-context {
  padding: 0 22px 6px;
  display: flex;
  align-items: flex-start;
  flex-wrap: wrap;
  gap: 10px 16px;
  animation: cueFade 0.25s ease;
}

@keyframes cueFade {
  from { opacity: 0; transform: translateY(6px); }
  to   { opacity: 1; transform: translateY(0); }
}

.answer-left {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  flex: 1;
  min-width: 300px;
}

.answer-icon {
  font-size: 14px;
  color: var(--accent, #f5c518);
  margin-top: 1px;
  flex: none;
}

.answer-text {
  display: flex;
  flex-direction: column;
}

.answer-line {
  font-size: 14.5px;
  line-height: 1.45;
  color: var(--text-primary, #e9ebf0);
}

.answer-sub {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10.5px;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--text-faint, #5f6570);
  margin-top: 4px;
}
</style>
