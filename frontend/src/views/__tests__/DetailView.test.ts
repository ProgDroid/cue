import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import DetailView from '../DetailView.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import type { Title } from '@/types'

const title: Title = {
  id: 5, imdbId: 'tt5', title: 'Coco', year: 2017, services: ['disney'],
  type: 'movie', genres: ['Animation', 'Musical'], imdb: 8.4, len: '105 min',
  desc: 'A boy and music.', cast: ['Anthony Gonzalez'], watched: false, rating: null,
}

beforeEach(() => setActivePinia(createPinia()))

describe('DetailView', () => {
  it('renders the routed title facts and cast', async () => {
    const router = createRouter({ history: createMemoryHistory(), routes: [
      { path: '/', component: { template: '<div>home</div>' } },
      { path: '/title/:id', component: DetailView },
    ] })
    const s = useCatalogueStore(); s.catalogue = [title]
    router.push('/title/5'); await router.isReady()
    const w = mount(DetailView, { global: { plugins: [router] } })
    await flushPromises()
    expect(w.text()).toContain('Coco')
    expect(w.text()).toContain('Anthony Gonzalez')
    expect(w.text()).toContain('2017')
  })
})
