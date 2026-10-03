import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import DetailView from '../DetailView.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import * as client from '@/api/client'
import { askService } from '@/services'
import type { Title, TitleDetail } from '@/types'

const listItem: Title = {
  id: 5, imdbId: 'tt5', title: 'Coco', year: 2017, services: ['disney'],
  type: 'movie', genres: ['Animation', 'Musical'], score: 8.4, anilistScore: null, len: '105 min',
  watched: false, rating: null, newSince: null,
}
const detail: TitleDetail = { ...listItem, desc: 'A boy and music.', cast: ['Anthony Gonzalez'], watchable: [] }

beforeEach(() => { setActivePinia(createPinia()); vi.restoreAllMocks() })

async function mountAt(id: number, catalogue: Title[] = [listItem]) {
  const router = createRouter({ history: createMemoryHistory(), routes: [
    { path: '/', component: { template: '<div>home</div>' } },
    { path: '/title/:id', component: DetailView },
  ] })
  const s = useCatalogueStore(); s.catalogue = catalogue
  router.push(`/title/${id}`); await router.isReady()
  const w = mount(DetailView, { global: { plugins: [router] } })
  await flushPromises()
  return w
}

describe('DetailView', () => {
  it('renders the fetched title facts and cast', async () => {
    vi.spyOn(client, 'getTitle').mockResolvedValue(detail)
    const w = await mountAt(5)
    expect(w.text()).toContain('Coco')
    expect(w.text()).toContain('Anthony Gonzalez')
    expect(w.text()).toContain('2017')
    expect(w.find('.poster img').attributes('src')).toContain('/poster')
    expect(w.find('.backdrop img').attributes('src')).toContain('/backdrop')
  })

  it('shows a not-found panel on 404', async () => {
    vi.spyOn(client, 'getTitle').mockRejectedValue(new client.NotFoundError('nope'))
    const w = await mountAt(999)
    expect(w.text()).toContain('Title not found')
  })

  it('shows a retry panel on a transient (non-404) error, and retry recovers', async () => {
    const spy = vi.spyOn(client, 'getTitle').mockRejectedValue(new Error('network down'))
    const w = await mountAt(5)
    expect(w.find('[data-test="detail-error"]').exists()).toBe(true)
    expect(w.text()).not.toContain('Title not found')
    // Retry re-invokes getTitle; on success the detail renders.
    spy.mockResolvedValueOnce(detail)
    await w.find('[data-test="detail-error"] button').trigger('click')
    await flushPromises()
    expect(w.text()).toContain('Coco')
  })

  it('shows both AniList and generic Rating pills when both scores are present', async () => {
    const detailWithScores: TitleDetail = { ...detail, score: 7.4, anilistScore: 8.6 }
    vi.spyOn(client, 'getTitle').mockResolvedValue(detailWithScores)
    const w = await mountAt(5)
    expect(w.find('[data-test="detail-anilist"]').exists()).toBe(true)
    expect(w.find('[data-test="detail-score"]').exists()).toBe(true)
    expect(w.text()).toContain('8.6')
    expect(w.text()).toContain('7.4')
  })

  it('re-fetches when the route id changes', async () => {
    const detail6: TitleDetail = { ...listItem, id: 6, title: 'Up', desc: 'A balloon.', cast: ['Ed Asner'], watchable: [] }
    vi.spyOn(client, 'getTitle').mockImplementation((id) =>
      id === 6 ? Promise.resolve(detail6) : Promise.resolve(detail),
    )
    const router = createRouter({ history: createMemoryHistory(), routes: [
      { path: '/', component: { template: '<div>home</div>' } },
      { path: '/title/:id', component: DetailView },
    ] })
    const s = useCatalogueStore(); s.catalogue = [listItem]
    router.push('/title/5'); await router.isReady()
    mount(DetailView, { global: { plugins: [router] } })
    await flushPromises()
    router.push('/title/6')
    await flushPromises()
    expect(client.getTitle).toHaveBeenCalledWith(6)
  })

  describe('similar titles strip', () => {
    const others: Title[] = [
      { ...listItem, id: 6, title: 'Up', genres: ['Animation', 'Musical'], score: 8.3 },
      { ...listItem, id: 7, title: 'Soul', genres: ['Animation'], score: 8.0 },
      { ...listItem, id: 8, title: 'Heat', genres: ['Crime'], score: 8.3 },
    ]
    const stripTitles = (w: Awaited<ReturnType<typeof mountAt>>) =>
      w.findAll('.sim-card').map(c => c.attributes('aria-label'))

    it('shows the server-ranked (embedding) order, skipping unknown ids', async () => {
      vi.spyOn(client, 'getTitle').mockResolvedValue(detail)
      const spy = vi.spyOn(askService, 'similar').mockResolvedValue({
        line: '', sub: '', ids: [8, 999, 7, 6],
      })
      const w = await mountAt(5, [listItem, ...others])
      expect(spy).toHaveBeenCalledWith(expect.objectContaining({ id: 5 }), expect.anything())
      expect(stripTitles(w)).toEqual(['Open Heat', 'Open Soul', 'Open Up'])
    })

    it('falls back to local genre ranking when the server call fails', async () => {
      vi.spyOn(client, 'getTitle').mockResolvedValue(detail)
      vi.spyOn(askService, 'similar').mockRejectedValue(new Error('ask failed (HTTP 500)'))
      const w = await mountAt(5, [listItem, ...others])
      expect(stripTitles(w)).toEqual(['Open Up', 'Open Soul'])
    })

    it('ignores a stale response from a previous title', async () => {
      const detail6: TitleDetail = { ...others[0], desc: '', cast: [], watchable: [] }
      vi.spyOn(client, 'getTitle').mockImplementation((id) =>
        Promise.resolve(id === 6 ? detail6 : detail))
      let releaseFirst: (v: { line: string; sub: string; ids: number[] }) => void = () => {}
      vi.spyOn(askService, 'similar').mockImplementation((t) => t.id === 5
        ? new Promise(r => { releaseFirst = r })
        : Promise.resolve({ line: '', sub: '', ids: [8] }))
      const router = createRouter({ history: createMemoryHistory(), routes: [
        { path: '/', component: { template: '<div>home</div>' } },
        { path: '/title/:id', component: DetailView },
      ] })
      const s = useCatalogueStore(); s.catalogue = [listItem, ...others]
      router.push('/title/5'); await router.isReady()
      const w = mount(DetailView, { global: { plugins: [router] } })
      await flushPromises()
      router.push('/title/6'); await flushPromises()
      releaseFirst({ line: '', sub: '', ids: [7] }); await flushPromises()
      expect(stripTitles(w)).toEqual(['Open Heat'])
    })
  })
})
