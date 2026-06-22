<script setup lang="ts">
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useCatalogueStore } from '@/stores/catalogue'
import FilterBar from '@/components/FilterBar.vue'
import PosterGrid from '@/components/PosterGrid.vue'
import type { Title } from '@/types'

const store = useCatalogueStore()
const router = useRouter()

onMounted(() => {
  if (store.status === 'idle') store.load()
})

function openDetail(id: number) {
  void router.push(`/title/${id}`)
}

// find-similar wired in Task 15
function onFindSimilar(_t: Title) {
  // no-op until Task 15
}
</script>

<template>
  <div class="browse-view">
    <FilterBar />

    <PosterGrid
      v-if="store.visibleTitles.length > 0"
      :titles="store.visibleTitles"
      @select="openDetail"
      @find-similar="onFindSimilar"
    />

    <div v-else class="empty-state">
      Nothing in your library matches those filters.
    </div>
  </div>
</template>

<style scoped>
.browse-view {
  min-height: 100%;
}

.empty-state {
  padding: 60px 22px;
  text-align: center;
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
  font-size: var(--t-base, 13.5px);
  color: var(--text-faint, #5f6570);
}
</style>
