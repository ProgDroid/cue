import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import FilterBar from '../FilterBar.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import type { Title } from '@/types'

beforeEach(() => setActivePinia(createPinia()))

/** Minimal but fully-shaped Title stub — satisfies all store getters. */
function t(id: number): Title {
  return {
    id,
    imdbId: null,
    title: '',
    year: 2000,
    services: ['plex'],
    type: 'movie',
    genres: [],
    score: null,
    anilistScore: null,
    len: '',
    watched: false,
    rating: null,
  }
}

describe('FilterBar', () => {
  it('clicking Plex sets store.service and marks the button active', async () => {
    const w = mount(FilterBar)
    const s = useCatalogueStore()
    await w.get('[data-test="service-plex"]').trigger('click')
    expect(s.service).toBe('plex')
    expect(w.get('[data-test="service-plex"]').classes()).toContain('active')
  })

  it('shows the result count from visibleTitles', () => {
    const s = useCatalogueStore()
    s.catalogue = [t(1), t(2)]
    const w = mount(FilterBar)
    expect(w.get('[data-test="count"]').text()).toMatch(/2 titles/)
  })

  it('renders the rating threshold options', () => {
    const w = mount(FilterBar)
    const opts = w.get('[data-test="rating-select"]').findAll('option').map(o => o.text())
    expect(opts).toEqual(['Any rating', '6+', '7+', '8+', '9+'])
  })

  it('selecting 8+ sets store.minRating to 8', async () => {
    const w = mount(FilterBar)
    const s = useCatalogueStore()
    await w.get('[data-test="rating-select"]').setValue('8')
    expect(s.minRating).toBe(8)
  })
})
