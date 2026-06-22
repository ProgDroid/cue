import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import DetailView from '../DetailView.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import type { Title } from '@/types'

const title: Title = {
  id: 5,
  imdbId: 'tt0000005',
  title: 'Coco',
  year: 2017,
  services: ['disney'],
  type: 'movie',
  genres: ['Animation', 'Musical'],
  imdb: 8.4,
  len: '105 min',
  desc: 'A boy and music.',
  cast: ['Anthony Gonzalez'],
  watched: false,
  rating: null,
}

beforeEach(() => setActivePinia(createPinia()))

describe('DetailView — watched/rating store wiring', () => {
  async function mountDetail() {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/', component: { template: '<div>home</div>' } },
        { path: '/title/:id', component: DetailView },
      ],
    })
    const store = useCatalogueStore()
    store.catalogue = [title]
    router.push('/title/5')
    await router.isReady()
    const wrapper = mount(DetailView, { global: { plugins: [router] } })
    await flushPromises()
    return { wrapper, store }
  }

  it('toggleWatched uses the numeric id, not a ref object', async () => {
    const { wrapper, store } = await mountDetail()

    const btn = wrapper.find('[data-test="mark-watched"]')
    expect(btn.exists()).toBe(true)

    await btn.trigger('click')

    // Must be keyed by the real numeric id 5, not "[object Object]"
    expect(store.isWatched(5)).toBe(true)
    // No "[object Object]" pollution
    const keys = Object.keys(store.watched)
    expect(keys).not.toContain('[object Object]')
  })

  it('setRating uses the numeric id, not a ref object', async () => {
    const { wrapper, store } = await mountDetail()

    const stars = wrapper.findAll('[data-test="star"]')
    expect(stars).toHaveLength(5)

    // Click the 4th star (index 3)
    await stars[3].trigger('click')

    // Must be keyed by the real numeric id 5
    expect(store.ratingOf(5)).toBe(4)
    // No "[object Object]" pollution
    const keys = Object.keys(store.ratings)
    expect(keys).not.toContain('[object Object]')
  })
})
