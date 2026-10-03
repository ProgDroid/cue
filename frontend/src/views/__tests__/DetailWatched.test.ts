import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import DetailView from '../DetailView.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import * as client from '@/api/client'
import type { TitleDetail } from '@/types'

vi.mock('@/api/userData', () => ({
  setRating: vi.fn().mockResolvedValue({ rating: 7 }),
  clearRating: vi.fn().mockResolvedValue({ rating: null }),
  setWatched: vi.fn().mockResolvedValue({ watched: true }),
}))

function makeDetail(over: Partial<TitleDetail> = {}): TitleDetail {
  return {
    id: 5, imdbId: 'tt0000005', title: 'Coco', year: 2017, services: ['disney'],
    type: 'movie', genres: ['Animation'], score: 8.4, anilistScore: null, len: '105 min',
    desc: 'A boy and music.', cast: ['Anthony Gonzalez'], watched: false, rating: null, newSince: null,
    watchable: [],
    ...over,
  }
}

beforeEach(() => { setActivePinia(createPinia()); vi.restoreAllMocks() })

async function mountDetail(title: TitleDetail) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: { template: '<div>home</div>' } },
      { path: '/title/:id', component: DetailView },
    ],
  })
  vi.spyOn(client, 'getTitle').mockResolvedValue(title)
  const store = useCatalogueStore()
  store.catalogue = [title]
  router.push(`/title/${title.id}`)
  await router.isReady()
  const wrapper = mount(DetailView, { global: { plugins: [router] } })
  await flushPromises()
  return { wrapper, store }
}

describe('DetailView — watched/rating store wiring', () => {
  it('mark-watched persists and updates the store', async () => {
    const { wrapper, store } = await mountDetail(makeDetail())
    await wrapper.find('[data-test="mark-watched"]').trigger('click')
    await flushPromises()
    expect(store.isWatched(5)).toBe(true)
    expect(Object.keys(store.watched)).not.toContain('[object Object]')
  })

  it('clicking a star persists the rating', async () => {
    const { wrapper, store } = await mountDetail(makeDetail())
    const stars = wrapper.findAll('[data-test="star"]')
    expect(stars).toHaveLength(10)
    await stars[6].trigger('click') // 7th pip
    await flushPromises()
    expect(store.ratingOf(5)).toBe(7)
    expect(Object.keys(store.ratings)).not.toContain('[object Object]')
  })

  it('surfaces a userDataError from the store near the controls', async () => {
    const { wrapper, store } = await mountDetail(makeDetail())
    expect(wrapper.find('[data-test="userdata-error"]').exists()).toBe(false)
    store.userDataError = 'No IMDb match — rating not saved.'
    await flushPromises()
    const el = wrapper.find('[data-test="userdata-error"]')
    expect(el.exists()).toBe(true)
    expect(el.text()).toContain('No IMDb match')
  })

  it('disables controls when the title has no imdbId', async () => {
    const { wrapper } = await mountDetail(makeDetail({ imdbId: null }))
    expect(wrapper.find('[data-test="mark-watched"]').attributes('disabled')).toBeDefined()
    expect(wrapper.find('[data-test="star"]').attributes('disabled')).toBeDefined()
    expect(wrapper.find('.no-imdb-hint').exists()).toBe(true)
  })
})
