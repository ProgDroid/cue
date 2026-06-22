<template>
  <header class="app-header">
    <!-- Wordmark -->
    <div class="wordmark" @click="$router.push('/')">
      <span class="wordmark__text">cue</span>
      <span class="wordmark__dot" aria-hidden="true"></span>
    </div>

    <!-- Eyebrow separator + label -->
    <span class="eyebrow">self-hosted library</span>

    <!-- Centered search input -->
    <div class="search-wrap">
      <span class="search-glyph" aria-hidden="true">⌕</span>
      <input
        data-test="search"
        type="search"
        v-model="query"
        placeholder="Search titles"
        class="search-input"
        autocomplete="off"
        spellcheck="false"
      />
    </div>

    <!-- Avatar -->
    <div class="avatar" aria-label="User: FF">FF</div>
  </header>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useCatalogueStore } from '@/stores/catalogue'

const store = useCatalogueStore()
const query = computed({
  get: () => store.query,
  set: (v: string) => store.setQuery(v),
})
</script>

<style scoped>
.app-header {
  flex: none;
  height: var(--header-h, 58px);
  display: flex;
  align-items: center;
  gap: 20px;
  padding: 0 22px;
  border-bottom: 1px solid var(--border-subtle, rgba(255, 255, 255, 0.07));
  background: rgba(11, 12, 15, 0.92);
  backdrop-filter: blur(8px);
  z-index: 30;
  position: sticky;
  top: 0;
}

/* Wordmark */
.wordmark {
  display: flex;
  align-items: baseline;
  gap: 2px;
  cursor: pointer;
  flex: none;
}

.wordmark__text {
  font-size: var(--t-wordmark, 21px);
  font-weight: 800;
  letter-spacing: -0.04em;
  color: var(--text-strong, #f4f5f7);
  font-family: var(--font-ui);
  line-height: 1;
}

.wordmark__dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent, #f5c518);
  display: inline-block;
  margin-left: 1px;
  transform: translateY(-1px);
  flex: none;
}

/* Eyebrow */
.eyebrow {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: var(--t-label, 10px);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faintest, #4f555f);
  padding-left: 14px;
  border-left: 1px solid var(--border, rgba(255, 255, 255, 0.08));
  white-space: nowrap;
  flex: none;
}

/* Search */
.search-wrap {
  flex: 1;
  max-width: 440px;
  margin: 0 auto;
  position: relative;
}

.search-glyph {
  position: absolute;
  left: 12px;
  top: 50%;
  transform: translateY(-50%);
  color: var(--text-faint, #5f6570);
  font-size: 14px;
  pointer-events: none;
  line-height: 1;
}

.search-input {
  width: 100%;
  height: 36px;
  background: var(--surface-1, #15171c);
  border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
  border-radius: var(--r-md, 8px);
  color: var(--text-primary, #e9ebf0);
  font-family: var(--font-ui, 'Hanken Grotesk', sans-serif);
  font-size: 13.5px;
  padding: 0 12px 0 32px;
  outline: none;
  /* Remove browser default search cancel button */
  -webkit-appearance: none;
  appearance: none;
}

.search-input::-webkit-search-cancel-button {
  display: none;
}

.search-input::placeholder {
  color: var(--text-faint, #5f6570);
}

.search-input:focus {
  border-color: var(--accent-focus, rgba(245, 197, 24, 0.5));
}

/* Avatar */
.avatar {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  background: linear-gradient(135deg, #2a2e37, #1a1d23);
  border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
  display: flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 12px;
  font-weight: 600;
  color: var(--text-muted, #aab0bb);
  flex: none;
  user-select: none;
}
</style>
