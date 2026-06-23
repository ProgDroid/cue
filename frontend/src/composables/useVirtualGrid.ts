import { ref, computed, onMounted, onUnmounted, watch, type Ref, type ComputedRef } from 'vue'
import { computeWindow } from './computeWindow'

// Persists across remounts (back-nav from DetailView) so spacer height is
// correct on the first frame and the browser can restore scroll position.
let cachedRowHeight = 0

export interface UseVirtualGridOptions {
  containerEl: Ref<HTMLElement | null>
  rowHeight: Ref<number>
  itemCount: Ref<number>
  overscanRows?: number
}

export interface UseVirtualGrid {
  cols: ComputedRef<number>
  startIndex: ComputedRef<number>
  endIndex: ComputedRef<number>
  topSpacer: ComputedRef<number>
  bottomSpacer: ComputedRef<number>
}

export function useVirtualGrid(opts: UseVirtualGridOptions): UseVirtualGrid {
  const overscanRows = opts.overscanRows ?? 3
  const containerWidth = ref(0)
  const scrollY = ref(0)
  const viewportH = ref(0)
  const gridTop = ref(0)

  function readScroll() {
    scrollY.value = window.scrollY
    viewportH.value = window.innerHeight
    const el = opts.containerEl.value
    gridTop.value = el ? el.getBoundingClientRect().top + window.scrollY : 0
  }

  let ro: ResizeObserver | null = null

  onMounted(() => {
    if (opts.rowHeight.value === 0 && cachedRowHeight > 0) {
      opts.rowHeight.value = cachedRowHeight
    }
    readScroll()
    const el = opts.containerEl.value
    if (el) {
      ro = new ResizeObserver((entries) => {
        containerWidth.value = entries[0].contentRect.width
        readScroll()
      })
      ro.observe(el)
      containerWidth.value = el.clientWidth
    }
    window.addEventListener('scroll', readScroll, { passive: true })
    window.addEventListener('resize', readScroll, { passive: true })
  })

  onUnmounted(() => {
    ro?.disconnect()
    window.removeEventListener('scroll', readScroll)
    window.removeEventListener('resize', readScroll)
  })

  watch(opts.rowHeight, (h) => { if (h > 0) cachedRowHeight = h })

  const win = computed(() => computeWindow({
    containerWidth: containerWidth.value,
    rowHeight: opts.rowHeight.value,
    scrollOffset: Math.max(0, scrollY.value - gridTop.value),
    viewportH: viewportH.value,
    itemCount: opts.itemCount.value,
    overscanRows,
  }))

  return {
    cols: computed(() => win.value.cols),
    startIndex: computed(() => win.value.startIndex),
    endIndex: computed(() => win.value.endIndex),
    topSpacer: computed(() => win.value.topSpacer),
    bottomSpacer: computed(() => win.value.bottomSpacer),
  }
}
