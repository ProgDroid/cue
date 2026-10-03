<script setup lang="ts">
import { computed } from 'vue'
import { useCatalogueStore, type SortKey } from '@/stores/catalogue'
import { services } from '@/design/tokens'
import type { ServiceKey } from '@/types'

const store = useCatalogueStore()

// Service buttons: All + each service key
const serviceButtons = computed(() => [
  { key: 'all' as const, label: 'All', dot: null },
  ...Object.entries(services).map(([key, val]) => ({
    key: key as ServiceKey,
    label: val.label,
    dot: val.dot,
  })),
])

// Type buttons
const typeButtons = [
  { key: 'all' as const, label: 'All' },
  { key: 'movie' as const, label: 'Movies' },
  { key: 'series' as const, label: 'Series' },
]

// Sort options — Relevance only exists while an answer is active
const sortOptions = computed<{ value: SortKey; label: string }[]>(() => [
  ...(store.answerActive ? [{ value: 'relevance' as const, label: 'Relevance' }] : []),
  { value: 'trending', label: 'Trending' },
  { value: 'foryou', label: 'For you' },
  { value: 'rating', label: 'Top rated' },
  { value: 'year', label: 'Newest' },
  { value: 'az', label: 'A–Z' },
])

// Rating threshold options (value 0 = no filter)
const ratingOptions = [
  { value: 0, label: 'Any rating' },
  { value: 6, label: '6+' },
  { value: 7, label: '7+' },
  { value: 8, label: '8+' },
  { value: 9, label: '9+' },
]

// Genre options derived from store
const genreOptions = computed(() => [
  { value: 'all', label: 'All genres' },
  ...store.genres.map(g => ({ value: g, label: g })),
])

// Two-way bindings for selects
const selectedGenre = computed({
  get: () => store.genre,
  set: (v: string) => store.setGenre(v),
})
const selectedSort = computed({
  get: () => store.sort,
  set: (v: SortKey) => store.setSort(v),
})
const selectedRating = computed({
  get: () => store.minRating,
  set: (v: number) => store.setMinRating(Number(v)),
})

// Result count
const resultCount = computed(() => `${store.visibleTitles.length} titles`)
</script>

<template>
  <div class="filter-bar">
    <!-- Service segmented group -->
    <div class="seg-group">
      <button
        v-for="btn in serviceButtons"
        :key="btn.key"
        :data-test="'service-' + btn.key"
        :class="['seg-btn', { active: store.service === btn.key }]"
        @click="store.setService(btn.key)"
      >
        <span v-if="btn.dot" class="service-dot" :style="{ background: btn.dot }" aria-hidden="true"></span>
        {{ btn.label }}
      </button>
    </div>

    <!-- Type segmented group -->
    <div class="seg-group">
      <button
        v-for="btn in typeButtons"
        :key="btn.key"
        :data-test="'type-' + btn.key"
        :class="['seg-btn', { active: store.type === btn.key }]"
        @click="store.setType(btn.key)"
      >
        {{ btn.label }}
      </button>
    </div>

    <!-- Genre select -->
    <div class="select-wrap">
      <select v-model="selectedGenre" data-test="genre-select" class="filter-select" aria-label="Filter by genre">
        <option v-for="opt in genreOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
      </select>
      <span class="select-arrow" aria-hidden="true">&#9660;</span>
    </div>

    <!-- Sort select -->
    <div class="select-wrap">
      <select v-model="selectedSort" data-test="sort-select" class="filter-select" aria-label="Sort by">
        <option
          v-for="opt in sortOptions"
          :key="opt.value"
          :value="opt.value"
          :disabled="opt.value === 'foryou' && !store.forYouAvailable"
        >{{ opt.label }}</option>
      </select>
      <span class="select-arrow" aria-hidden="true">&#9660;</span>
    </div>
    <span v-if="store.forYouLocked" data-test="foryou-hint" class="sort-note">
      Rate 3+ titles you liked to unlock For you
    </span>
    <button
      v-if="store.forYou.status === 'error'"
      type="button"
      data-test="foryou-error"
      class="sort-note sort-note-error"
      @click="store.loadForYou()"
    >For you unavailable — retry</button>

    <!-- Rating threshold select -->
    <div class="select-wrap">
      <select v-model="selectedRating" data-test="rating-select" class="filter-select" aria-label="Filter by minimum rating">
        <option v-for="opt in ratingOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
      </select>
      <span class="select-arrow" aria-hidden="true">&#9660;</span>
    </div>

    <!-- Result count -->
    <div data-test="count" class="result-count">{{ resultCount }}</div>
  </div>
</template>

<style scoped>
.filter-bar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px 14px;
  padding: 18px 22px 14px;
}

/* Segmented control group */
.seg-group {
  display: flex;
  gap: 4px;
  padding: 3px;
  background: var(--surface-1, #15171c);
  border: 1px solid var(--border-subtle, rgba(255, 255, 255, 0.07));
  border-radius: 9px;
}

/* Segmented button (default = inactive) */
.seg-btn {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 11px;
  background: transparent;
  border: none;
  border-radius: 6px;
  color: var(--text-tertiary, #aab0bb);
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
  transition: background 120ms ease, color 120ms ease;
  white-space: nowrap;
}

.seg-btn:hover {
  color: var(--text-primary, #e9ebf0);
}

/* Active state */
.seg-btn.active {
  background: var(--surface-3, #1d2129);
  color: var(--text-strong, #f4f5f7);
}

/* Service colored dot */
.service-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex: none;
}

/* Select wrapper */
.select-wrap {
  position: relative;
}

.filter-select {
  appearance: none;
  -webkit-appearance: none;
  height: 34px;
  padding: 0 30px 0 12px;
  background: var(--surface-1, #15171c);
  border: 1px solid var(--border-subtle, rgba(255, 255, 255, 0.07));
  border-radius: 9px;
  color: var(--text-secondary, #c2c7d0);
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 12px;
  cursor: pointer;
  outline: none;
  transition: border-color 120ms ease;
}

.filter-select:focus {
  border-color: var(--accent-focus, rgba(245, 197, 24, 0.5));
}

.select-arrow {
  position: absolute;
  right: 11px;
  top: 50%;
  transform: translateY(-50%);
  pointer-events: none;
  color: var(--text-faint, #5f6570);
  font-size: 10px;
}

/* Sort-menu captions (For you hint / error) */
.sort-note {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 11px;
  letter-spacing: 0.04em;
  color: var(--text-muted, #8a909b);
}

.sort-note-error {
  padding: 0;
  background: none;
  border: none;
  color: var(--text-danger, #f5a3a3);
  cursor: pointer;
  text-decoration: underline;
}

/* Result count — right-aligned via margin-left: auto */
.result-count {
  margin-left: auto;
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 11px;
  letter-spacing: 0.04em;
  color: var(--text-faint, #5f6570);
  white-space: nowrap;
}
</style>
