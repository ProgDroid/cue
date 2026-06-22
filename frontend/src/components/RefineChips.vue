<script setup lang="ts">
import { useCatalogueStore } from '@/stores/catalogue'

const store = useCatalogueStore()
</script>

<template>
  <div class="refine-chips">
    <button
      data-test="refine-lighter"
      class="chip"
      @click="store.refine('lighter')"
    >
      Even lighter
    </button>
    <button
      data-test="refine-shorter"
      class="chip"
      @click="store.refine('shorter')"
    >
      Make it shorter
    </button>
    <button
      data-test="refine-surprise"
      class="chip"
      @click="store.refine('surprise')"
    >
      Surprise me
    </button>
  </div>
</template>

<style scoped>
.refine-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.chip {
  padding: 6px 13px;
  background: #1d2129;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 999px;
  color: var(--text-secondary, #d2d6dd);
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
  font-size: 12.5px;
  cursor: pointer;
  white-space: nowrap;
  transition: border-color 120ms ease, color 120ms ease;
}

.chip:hover {
  border-color: rgba(245, 197, 24, 0.4);
  color: var(--text-primary, #f4f5f7);
}
</style>
