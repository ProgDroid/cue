import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import BrowseView from '../BrowseView.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))

describe('Browse keyboard', () => {
  it('Esc clears an active answer', async () => {
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue([])
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: BrowseView }] })
    router.push('/'); await router.isReady()
    const w = mount(BrowseView, { attachTo: document.body, global: { plugins: [router] } })
    await flushPromises()
    const s = useCatalogueStore(); s.answerActive = true; s.thread = [{ label: 'x', line: '', sub: '', ids: [] }]
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(s.answerActive).toBe(false)
    w.unmount()
  })
})
