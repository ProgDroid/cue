import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
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
    newSince: null,
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

  it('each filter select has an aria-label', () => {
    const w = mount(FilterBar)
    expect(w.get('[data-test="genre-select"]').attributes('aria-label')).toBe('Filter by genre')
    expect(w.get('[data-test="sort-select"]').attributes('aria-label')).toBe('Sort by')
    expect(w.get('[data-test="rating-select"]').attributes('aria-label')).toBe('Filter by minimum rating')
  })

  describe('sort menu', () => {
    const sortOpts = (w: ReturnType<typeof mount>) =>
      w.get('[data-test="sort-select"]').findAll('option')

    it('browse mode lists Trending, For you, Top rated, Newest, A–Z (no Relevance)', () => {
      const w = mount(FilterBar)
      expect(sortOpts(w).map(o => o.text())).toEqual(['Trending', 'For you', 'Top rated', 'Newest', 'A–Z'])
      expect(sortOpts(w).map(o => o.element.value)).toEqual(['trending', 'foryou', 'rating', 'year', 'az'])
    })

    it('answer mode adds Relevance as the first option', () => {
      const s = useCatalogueStore()
      s.answerActive = true
      s.sort = 'relevance'
      const w = mount(FilterBar)
      const opts = sortOpts(w)
      expect(opts.map(o => o.text())).toEqual(['Relevance', 'Trending', 'For you', 'Top rated', 'Newest', 'A–Z'])
      expect(opts[0].element.value).toBe('relevance')
      expect((w.get('[data-test="sort-select"]').element as HTMLSelectElement).value).toBe('relevance')
    })

    it('For you is disabled with the unlock hint below basis 3', () => {
      const s = useCatalogueStore()
      s.forYou = { status: 'idle', ids: [], basis: 2 }
      const w = mount(FilterBar)
      const opt = w.get('option[value="foryou"]').element as HTMLOptionElement
      expect(opt.disabled).toBe(true)
      expect(w.get('[data-test="foryou-hint"]').text()).toBe('Rate 3+ titles you liked to unlock For you')
    })

    it('For you is enabled and the hint hidden at basis 3', () => {
      const s = useCatalogueStore()
      s.forYou = { status: 'ready', ids: [], basis: 3 }
      const w = mount(FilterBar)
      expect((w.get('option[value="foryou"]').element as HTMLOptionElement).disabled).toBe(false)
      expect(w.find('[data-test="foryou-hint"]').exists()).toBe(false)
    })

    it('shows "For you unavailable — retry" when for-you errored with For you selected, and retry calls loadForYou', async () => {
      const s = useCatalogueStore()
      s.forYou = { status: 'error', ids: [], basis: 5 }
      s.sort = 'foryou'
      const spy = vi.spyOn(s, 'loadForYou').mockResolvedValue(undefined)
      const w = mount(FilterBar)
      const err = w.get('[data-test="foryou-error"]')
      expect(err.text()).toBe('For you unavailable — retry')
      await err.trigger('click')
      expect(spy).toHaveBeenCalledTimes(1)
    })

    it('hides the error line when For you is not the selected sort', () => {
      const s = useCatalogueStore()
      s.forYou = { status: 'error', ids: [], basis: 5 }
      s.sort = 'trending'
      const w = mount(FilterBar)
      expect(w.find('[data-test="foryou-error"]').exists()).toBe(false)
    })
  })
})
