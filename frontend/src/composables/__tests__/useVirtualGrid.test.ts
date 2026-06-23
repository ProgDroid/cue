import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, afterEach, vi } from 'vitest'
import { defineComponent, h, ref, type Ref } from 'vue'
import { useVirtualGrid } from '@/composables/useVirtualGrid'

// Minimal host component: the composable relies on onMounted/onUnmounted +
// a container element, so it can only run inside a mounted component.
function harness(rowHeight: Ref<number>, itemCount = 100) {
  return defineComponent({
    setup() {
      const containerEl = ref<HTMLElement | null>(null)
      const vg = useVirtualGrid({ containerEl, rowHeight, itemCount: ref(itemCount) })
      return { containerEl, vg }
    },
    render() {
      return h('div', { ref: 'containerEl' })
    },
  })
}

afterEach(() => {
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 0 })
  vi.restoreAllMocks()
})

describe('useVirtualGrid', () => {
  it('derives cols from the measured container width', async () => {
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 800 })
    const w = mount(harness(ref(172)), { attachTo: document.body })
    await flushPromises()
    // 800px wide fits several columns; a 0-width (unmeasured) container falls to 1.
    expect((w.vm as unknown as { vg: { cols: { value: number } } }).vg.cols.value).toBeGreaterThan(1)
  })

  it('seeds rowHeight from the cross-remount cache on a later mount', async () => {
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 800 })
    // First mount measures: rowHeight goes 0 -> 172, populating the module cache.
    const rh1 = ref(0)
    const w1 = mount(harness(rh1), { attachTo: document.body })
    await flushPromises()
    rh1.value = 172
    await flushPromises()
    w1.unmount()

    // A fresh mount with an unmeasured rowHeight (0) is seeded from the cache so
    // the spacer is correct on the first frame (back-nav scroll restore).
    const rh2 = ref(0)
    mount(harness(rh2), { attachTo: document.body })
    await flushPromises()
    expect(rh2.value).toBe(172)
  })

  it('removes its window scroll/resize listeners on unmount', async () => {
    const remove = vi.spyOn(window, 'removeEventListener')
    const w = mount(harness(ref(172)), { attachTo: document.body })
    await flushPromises()
    w.unmount()
    expect(remove).toHaveBeenCalledWith('scroll', expect.any(Function))
    expect(remove).toHaveBeenCalledWith('resize', expect.any(Function))
  })
})
