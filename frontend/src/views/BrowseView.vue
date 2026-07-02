<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { useCatalogueStore } from '@/stores/catalogue'
import { useDelayedFlag } from '@/composables/useDelayedFlag'
import AskBar from '@/components/AskBar.vue'
import ThreadBreadcrumb from '@/components/ThreadBreadcrumb.vue'
import FilterBar from '@/components/FilterBar.vue'
import PosterGrid from '@/components/PosterGrid.vue'
import AnswerContext from '@/components/AnswerContext.vue'
import ShimmerGrid from '@/components/ShimmerGrid.vue'

const store = useCatalogueStore()
const showLoader = useDelayedFlag(() => store.status === 'loading', 180)
const router = useRouter()

function onKey(e: KeyboardEvent) {
  const typing = e.target instanceof HTMLElement && ['INPUT', 'TEXTAREA', 'SELECT'].includes(e.target.tagName)
  if (e.key === '/' && !typing) { e.preventDefault(); document.querySelector<HTMLInputElement>('[data-test="ask-input"]')?.focus() }
  else if (e.key === 'Escape') { if (store.answerActive) store.clearThread(); else (document.activeElement as HTMLElement | null)?.blur() }
}

onMounted(() => {
  if (store.status === 'idle') store.load()
  document.addEventListener('keydown', onKey)
})

onUnmounted(() => document.removeEventListener('keydown', onKey))

function openDetail(id: number) {
  void router.push(`/title/${id}`)
}

const emptyCopy = computed(() =>
  store.answerActive
    ? 'Nothing in this result set matches those filters — loosen a filter or clear the thread.'
    : 'Nothing in your library matches those filters.',
)
</script>

<template>
  <div class="browse-view">
    <div class="ask-dock">
      <AskBar />
      <ThreadBreadcrumb />
    </div>

    <FilterBar />

    <ShimmerGrid v-if="store.resolving || showLoader" data-test="shimmer" />
    <template v-else>
      <AnswerContext v-if="store.answerActive" />

      <PosterGrid
        v-if="store.visibleTitles.length > 0"
        :titles="store.visibleTitles"
        @select="openDetail"
        @find-similar="store.moreLike"
      />

      <div v-else-if="store.status === 'error'" class="empty-state" data-test="load-error">
        {{ store.error }} — <button class="retry-btn" @click="store.load()">Retry</button>
      </div>

      <div v-else-if="store.status === 'ready'" class="empty-state">
        {{ emptyCopy }}
      </div>
    </template>
  </div>
</template>

<style scoped>
.browse-view {
  min-height: 100%;
}

.ask-dock {
  position: sticky;
  top: 0;
  z-index: 22;
  padding: 18px 22px 12px;
  background: linear-gradient(180deg, #0b0c0f 76%, rgba(11, 12, 15, 0));
}

.empty-state {
  padding: 60px 22px;
  text-align: center;
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
  font-size: var(--t-base, 13.5px);
  color: var(--text-faint, #5f6570);
}

.retry-btn {
  background: none;
  border: none;
  color: var(--accent-text, #f5d24e);
  cursor: pointer;
  font: inherit;
  text-decoration: underline;
  padding: 0;
}
</style>
