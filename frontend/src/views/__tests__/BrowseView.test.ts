import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import BrowseView from '../BrowseView.vue'
import type { Title } from '@/types'

const routes = [
  { path: '/', component: BrowseView },
  { path: '/title/:id', component: { template: '<div>detail</div>' } },
]

beforeEach(() => setActivePinia(createPinia()))

function makeRouter() { return createRouter({ history: createMemoryHistory(), routes }) }

function t(id: number, title: string): Title {
  return { id, title, services: ['plex'], type: 'movie', genres: [], score: null, anilistScore: null, year: 2000, len: '', watched: false, rating: null, imdbId: null }
}

describe('BrowseView', () => {
  it('loads catalogue on mount and renders cards', async () => {
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue([
      t(1, 'A'), t(2, 'B'),
    ])
    const router = makeRouter(); router.push('/'); await router.isReady()
    const w = mount(BrowseView, { global: { plugins: [router] } })
    await flushPromises()
    expect(w.findAll('[data-test="card"]').length).toBe(2)
  })

  it('shows plain empty-state copy when no titles match', async () => {
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue([])
    const router = makeRouter(); router.push('/'); await router.isReady()
    const w = mount(BrowseView, { global: { plugins: [router] } })
    await flushPromises()
    expect(w.text()).toContain('Nothing in your library matches those filters')
  })
})
