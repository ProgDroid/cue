import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import AppHeader from '../AppHeader.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))
afterEach(() => vi.useRealTimers())

describe('AppHeader', () => {
  it('typing in search updates store.query', async () => {
    vi.useFakeTimers()
    const w = mount(AppHeader)
    const s = useCatalogueStore()
    await w.get('input[type="search"], input[data-test="search"]').setValue('frieren')
    vi.advanceTimersByTime(120)
    expect(s.query).toBe('frieren')
  })

  it('renders the wordmark', () => {
    const w = mount(AppHeader)
    expect(w.text()).toContain('cue')
  })

  it('debounces search input into the store (no call until the delay)', async () => {
    vi.useFakeTimers()
    const s = useCatalogueStore()
    const spy = vi.spyOn(s, 'setQuery')
    const w = mount(AppHeader)
    await w.find('[data-test="search"]').setValue('alien')
    expect(spy).not.toHaveBeenCalled()
    vi.advanceTimersByTime(120)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith('alien')
  })

  it('drops a pending debounced query when unmounted', async () => {
    vi.useFakeTimers()
    const s = useCatalogueStore()
    const spy = vi.spyOn(s, 'setQuery')
    const w = mount(AppHeader)
    await w.find('[data-test="search"]').setValue('alien')
    w.unmount()
    vi.advanceTimersByTime(200)
    expect(spy).not.toHaveBeenCalled()
  })
})
