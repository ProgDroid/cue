import { mount } from '@vue/test-utils'
import { describe, it, expect, vi, afterEach } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import { useDelayedFlag } from '@/composables/useDelayedFlag'

// The composable uses onUnmounted, so it must run inside a mounted component.
function harness(delay: number) {
  const source = ref(false)
  const cmp = defineComponent({
    setup() {
      const flag = useDelayedFlag(source, delay)
      return { flag }
    },
    render() {
      return h('span', this.flag ? 'on' : 'off')
    },
  })
  return { source, cmp }
}

afterEach(() => vi.useRealTimers())

describe('useDelayedFlag', () => {
  it('does not raise the flag before the delay elapses', async () => {
    vi.useFakeTimers()
    const { source, cmp } = harness(180)
    const w = mount(cmp)
    source.value = true
    await w.vm.$nextTick()
    vi.advanceTimersByTime(179)
    await w.vm.$nextTick()
    expect(w.text()).toBe('off')
  })

  it('raises the flag once the delay elapses', async () => {
    vi.useFakeTimers()
    const { source, cmp } = harness(180)
    const w = mount(cmp)
    source.value = true
    await w.vm.$nextTick()
    vi.advanceTimersByTime(180)
    await w.vm.$nextTick()
    expect(w.text()).toBe('on')
  })

  it('cancels the pending flag if the source clears before the delay', async () => {
    vi.useFakeTimers()
    const { source, cmp } = harness(180)
    const w = mount(cmp)
    source.value = true
    await w.vm.$nextTick()
    vi.advanceTimersByTime(100)
    source.value = false
    await w.vm.$nextTick()
    vi.advanceTimersByTime(200)
    await w.vm.$nextTick()
    expect(w.text()).toBe('off')
  })

  it('accepts a getter source as well as a ref', async () => {
    vi.useFakeTimers()
    const state = ref(false)
    const cmp = defineComponent({
      setup() {
        const flag = useDelayedFlag(() => state.value, 180)
        return { flag }
      },
      render() {
        return h('span', this.flag ? 'on' : 'off')
      },
    })
    const w = mount(cmp)
    state.value = true
    await w.vm.$nextTick()
    vi.advanceTimersByTime(180)
    await w.vm.$nextTick()
    expect(w.text()).toBe('on')
    state.value = false
    await w.vm.$nextTick()
    expect(w.text()).toBe('off')
  })
})
