import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '../catalogue'
import type { Title } from '@/types'

// The singleton in @/services/index.ts is now ApiAskService (backed by fetch).
// Swap it back to the deterministic stub so these store-behaviour tests don't
// need a running server or a fetch mock.
vi.mock('@/services', async () => {
  const { StubAskService } = await import('@/services/askService')
  return { askService: new StubAskService() }
})

// Import the mocked module so we can spy on the singleton instance's methods.
// Must be after vi.mock (hoisting means the factory runs before this import).
const { askService: mockedAskService } = await import('@/services')

function t(p: Partial<Title>): Title {
  return { id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], score: null, anilistScore: null, len: '90 min',
    watched: false, rating: null, ...p }
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

describe('store ask error handling (503 / unavailable)', () => {
  it('submitAsk does not throw when askService rejects, sets askError, resets resolving', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    vi.spyOn(mockedAskService, 'ask').mockRejectedValueOnce(new Error('ask is unavailable'))
    await expect(s.submitAsk('anything')).resolves.toBeUndefined()
    expect(s.askError).toBeTruthy()
    expect(s.resolving).toBe(false)
  })

  it('submitAsk sets answerActive so the error message is visible', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    vi.spyOn(mockedAskService, 'ask').mockRejectedValueOnce(new Error('ask is unavailable'))
    await s.submitAsk('anything')
    expect(s.answerActive).toBe(true)
    expect(s.askError).toBe('ask is unavailable')
  })

  it('submitAsk resets askError to null on a successful follow-up call', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    vi.spyOn(mockedAskService, 'ask').mockRejectedValueOnce(new Error('ask is unavailable'))
    await s.submitAsk('anything')
    expect(s.askError).toBeTruthy()
    // Next call succeeds (spy is one-time) — error should clear
    await s.submitAsk('animation')
    expect(s.askError).toBeNull()
  })

  it('clearThread also clears askError', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    vi.spyOn(mockedAskService, 'ask').mockRejectedValueOnce(new Error('ask is unavailable'))
    await s.submitAsk('anything')
    s.clearThread()
    expect(s.askError).toBeNull()
    expect(s.answerActive).toBe(false)
  })
})
