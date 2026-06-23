<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import type { Title } from '@/types'
import { useCatalogueStore } from '@/stores/catalogue'
import { useVirtualGrid } from '@/composables/useVirtualGrid'
import { ROW_GAP } from '@/design/grid'
import PosterCard from '@/components/PosterCard.vue'

const props = defineProps<{ titles: Title[] }>()
const emit = defineEmits<{
  select: [id: number]
  'find-similar': [t: Title]
}>()

const store = useCatalogueStore()

const containerEl = ref<HTMLElement | null>(null)
const gridEl = ref<HTMLElement | null>(null)
const rowHeight = ref(0)
const itemCount = computed(() => props.titles.length)

const { cols, startIndex, endIndex, topSpacer, bottomSpacer } = useVirtualGrid({
  containerEl,
  rowHeight,
  itemCount,
})

const visible = computed(() => props.titles.slice(startIndex.value, endIndex.value))

function measureRow() {
  const card = gridEl.value?.firstElementChild as HTMLElement | null
  if (card) rowHeight.value = card.offsetHeight + ROW_GAP
}

let ro: ResizeObserver | null = null
onMounted(() => {
  if (gridEl.value) {
    ro = new ResizeObserver(() => measureRow())
    ro.observe(gridEl.value)
  }
  void nextTick(measureRow)
})
onUnmounted(() => ro?.disconnect())
// re-measure when the rendered set changes (e.g. filter narrows the grid)
watch(visible, () => void nextTick(measureRow))
</script>

<template>
  <div ref="containerEl" class="poster-grid-virtual">
    <div :style="{ height: topSpacer + 'px' }" />
    <div ref="gridEl" class="poster-grid" :style="{ '--cols': cols }">
      <PosterCard
        v-for="t in visible"
        :key="t.id"
        :title="t"
        :watched="store.isWatched(t.id)"
        @select="emit('select', $event)"
        @find-similar="emit('find-similar', $event)"
      />
    </div>
    <div :style="{ height: bottomSpacer + 'px' }" />
  </div>
</template>

<style scoped>
.poster-grid {
  display: grid;
  grid-template-columns: repeat(var(--cols, 1), minmax(0, 1fr));
  gap: 22px 18px;
  padding: 6px 22px 40px;
}
</style>
