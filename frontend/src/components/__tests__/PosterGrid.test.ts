import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
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

describe('PosterGrid virtualization', () => {
  it('renders only the visible window and sets --cols', async () => {
    // 4 columns at width 800; rowHeight 172; viewport 400 -> 6 rows -> 24 cards
    vi.stubGlobal('innerHeight', 400)
    Object.defineProperty(window, 'scrollY', { value: 0, configurable: true })

    const titles = Array.from({ length: 100 }, (_, i) => t(i + 1))
    const w = mount(PosterGrid, {
      props: { titles },
      attachTo: document.body,
    })

    // Force deterministic measurements: container width + card height.
    const container = w.element as HTMLElement
    Object.defineProperty(container, 'clientWidth', { value: 800, configurable: true })
    // every element reports offsetHeight 150 so rowHeight = 150 + ROW_GAP(22) = 172
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { value: 150, configurable: true })

    // Re-trigger measurement now that sizes are defined.
    window.dispatchEvent(new Event('resize'))
    await flushPromises()

    const grid = w.find('.poster-grid')
    expect(grid.attributes('style')).toContain('--cols: 4')
    expect(w.findAllComponents({ name: 'PosterCard' }).length).toBeLessThan(100)
  })

  it('renders all cards when the set is smaller than a viewport', async () => {
    const titles = Array.from({ length: 6 }, (_, i) => t(i + 1))
    const w = mount(PosterGrid, { props: { titles }, attachTo: document.body })
    await flushPromises()
    expect(w.findAllComponents({ name: 'PosterCard' }).length).toBe(6)
  })
})
