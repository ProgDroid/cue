import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '../catalogue'
import type { Title } from '@/types'

function t(p: Partial<Title>): Title {
  return { id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], imdb: null, len: '90 min', desc: '',
    cast: [], watched: false, rating: null, ...p }
}
const cat: Title[] = [
  t({ id: 1, title: 'Frieren', genres: ['Animation'] }),
  t({ id: 2, title: 'Coco', genres: ['Animation'] }),
  t({ id: 3, title: 'Alien', genres: ['Horror'] }),
]

beforeEach(() => setActivePinia(createPinia()))

describe('store ask actions', () => {
  it('submitAsk activates the answer, sets the base set, pushes a thread step', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    await s.submitAsk('animation')
    expect(s.answerActive).toBe(true)
    expect(s.visibleTitles.map(x => x.id).sort()).toEqual([1, 2])
    expect(s.thread.length).toBe(1)
  })

  it('filters compose within the answer set', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    await s.submitAsk('animation')
    s.setQuery('coco')
    expect(s.visibleTitles.map(x => x.id)).toEqual([2])
  })

  it('clearThread resets to the full catalogue', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    await s.submitAsk('animation'); s.clearThread()
    expect(s.answerActive).toBe(false)
    expect(s.visibleTitles).toHaveLength(3)
  })

  it('stepThread(i) restores that step and truncates later steps', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    await s.submitAsk('animation')      // step 0
    await s.refine('lighter')           // step 1
    s.stepThread(0)
    expect(s.thread.length).toBe(1)
    expect(s.resultIds.sort()).toEqual([1, 2])
  })
})
