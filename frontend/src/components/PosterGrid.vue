<script setup lang="ts">
import type { Title } from '@/types'
import { useCatalogueStore } from '@/stores/catalogue'
import PosterCard from '@/components/PosterCard.vue'

defineProps<{ titles: Title[] }>()

const emit = defineEmits<{
  select: [id: number]
  'find-similar': [t: Title]
}>()

const store = useCatalogueStore()
</script>

<template>
  <div class="poster-grid">
    <PosterCard
      v-for="t in titles"
      :key="t.id"
      :title="t"
      :watched="store.isWatched(t.id)"
      @select="emit('select', $event)"
      @find-similar="emit('find-similar', $event)"
    />
  </div>
</template>

<style scoped>
.poster-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(158px, 1fr));
  gap: 22px 18px;
  padding: 6px 22px 40px;
}
</style>
