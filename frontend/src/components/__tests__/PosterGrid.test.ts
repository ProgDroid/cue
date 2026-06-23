import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import PosterGrid from '@/components/PosterGrid.vue'
import type { Title } from '@/types'

function t(id: number): Title {
  return {
    id, imdbId: null, title: `T${id}`, year: 2000, services: ['plex'],
    type: 'movie', genres: [], imdb: null, len: '90 min', watched: false, rating: null,
  }
}

beforeEach(() => setActivePinia(createPinia()))

afterEach(() => {
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get: () => 0 })
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 0 })
  vi.restoreAllMocks()
})

describe('PosterGrid virtualization', () => {
  it('renders only the visible window and sets --cols', async () => {
    // 4 columns at width 800; rowHeight 172 (offsetHeight 150 + ROW_GAP 22);
    // viewport 400, scrollY 0, overscan 3 -> endRow = ceil(400/172)+3 = 6 rows -> 24 cards
    vi.stubGlobal('innerHeight', 400)
    Object.defineProperty(window, 'scrollY', { value: 0, configurable: true })

    // Seed measurements BEFORE mount so the RO mock picks them up during onMounted/observe().
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 800 })
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get: () => 150 })

    const titles = Array.from({ length: 100 }, (_, i) => t(i + 1))
    const w = mount(PosterGrid, {
      props: { titles },
      attachTo: document.body,
    })
    await flushPromises()

    const grid = w.find('.poster-grid')
    expect(grid.attributes('style')).toContain('--cols: 4')
    expect(w.findAllComponents({ name: 'PosterCard' }).length).toBe(24)
  })

  it('renders all cards when the set is smaller than a viewport', async () => {
    const titles = Array.from({ length: 6 }, (_, i) => t(i + 1))
    const w = mount(PosterGrid, { props: { titles }, attachTo: document.body })
    await flushPromises()
    expect(w.findAllComponents({ name: 'PosterCard' }).length).toBe(6)
  })
})
