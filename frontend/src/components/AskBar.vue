<script setup lang="ts">
import { ref } from 'vue'
import { useCatalogueStore } from '@/stores/catalogue'

const store = useCatalogueStore()
const inputValue = ref('')

function submit() {
  const val = inputValue.value.trim()
  if (!val) return
  void store.submitAsk(val)
  inputValue.value = ''
}

function onKeyDown(e: KeyboardEvent) {
  if (e.key === 'Enter') submit()
}
</script>

<template>
  <div class="ask-bar">
    <span class="ask-icon" aria-hidden="true">✦</span>
    <input
      v-model="inputValue"
      data-test="ask-input"
      class="ask-input"
      placeholder='Ask anything — "lighter after Frieren", "short and tense", "make me cry"'
      @keydown="onKeyDown"
    />
    <button
      data-test="ask-submit"
      class="ask-btn"
      :disabled="store.resolving"
      @click="submit"
    >
      Ask
    </button>
  </div>
</template>

<style scoped>
.ask-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  background: linear-gradient(180deg, #181b21, #131519);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 12px;
  padding: 9px 9px 9px 15px;
  box-shadow: 0 6px 22px rgba(0, 0, 0, 0.4);
}

.ask-icon {
  font-size: 16px;
  color: var(--accent, #f5c518);
  flex: none;
}

.ask-input {
  flex: 1;
  background: transparent;
  border: none;
  outline: none;
  color: var(--text-primary, #e9ebf0);
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
  font-size: 14.5px;
}

.ask-input::placeholder {
  color: var(--text-faint, #5f6570);
}

.ask-btn {
  flex: none;
  height: 34px;
  padding: 0 16px;
  background: var(--accent, #f5c518);
  border: none;
  border-radius: 8px;
  color: #1a1400;
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
  font-size: 13px;
  font-weight: 700;
  cursor: pointer;
  transition: opacity 120ms ease;
}

.ask-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.ask-btn:not(:disabled):hover {
  opacity: 0.88;
}
</style>
