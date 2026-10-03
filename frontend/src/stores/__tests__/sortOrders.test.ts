import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '../catalogue'
import type { Title, ForYouResult } from '@/types'

vi.mock('@/services', async () => {
  const { StubAskService } = await import('@/services/askService')
  return { askService: new StubAskService() }
})
vi.mock('@/api/forYou', () => ({ getForYou: vi.fn() }))
vi.mock('@/api/userData', () => ({
  setRating: vi.fn(),
  clearRating: vi.fn(),
  setWatched: vi.fn(),
}))
vi.mock('@/api/client', () => ({ getCatalogue: vi.fn() }))

import { getForYou } from '@/api/forYou'
import { setRating, clearRating, setWatched } from '@/api/userData'
import { getCatalogue } from '@/api/client'

function t(p: Partial<Title>): Title {
  return { id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], score: null, anilistScore: null, len: '90 min',
    watched: false, rating: null, newSince: null, ...p }
}

function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

const ids = (s: ReturnType<typeof useCatalogueStore>) => s.visibleTitles.map(x => x.id)

beforeEach(() => {
  setActivePinia(createPinia())
  vi.clearAllMocks()
  vi.mocked(getForYou).mockResolvedValue({ ids: [], basis: 0 })
})

describe('trending order', () => {
  it('new titles first by newSince desc, then external rating desc, nulls last, ties by id', () => {
    const s = useCatalogueStore()
    s.catalogue = [
      t({ id: 1, score: 5 }),
      t({ id: 2 }),
      t({ id: 3, newSince: 100, score: 1 }),
      t({ id: 4, newSince: 200 }),
      t({ id: 5, score: 9 }),
      t({ id: 6, anilistScore: 70 }), // 7.0
      t({ id: 7, score: 5 }),
    ]
    expect(s.sort).toBe('trending')
    expect(ids(s)).toEqual([4, 3, 5, 6, 1, 7, 2])
  })
})

describe('relevance and answers', () => {
  it('an active answer keeps engine order under Relevance', () => {
    const s = useCatalogueStore()
    s.catalogue = [t({ id: 1, score: 9 }), t({ id: 2, score: 1 }), t({ id: 3, score: 5 })]
    s.applyResult('q', { ids: [3, 1, 2], line: 'l', sub: 's' })
    expect(s.sort).toBe('relevance')
    expect(ids(s)).toEqual([3, 1, 2])
  })

  it('entering an answer selects relevance and clearThread restores the browse sort', () => {
    const s = useCatalogueStore()
    s.catalogue = [t({ id: 1 }), t({ id: 2 })]
    s.setSort('year')
    s.applyResult('q', { ids: [2, 1], line: '', sub: '' })
    expect(s.sort).toBe('relevance')
    s.clearThread()
    expect(s.sort).toBe('year')
  })

  it('entering an answer via an error branch also selects relevance', async () => {
    const s = useCatalogueStore()
    s.catalogue = [t({ id: 1 })]
    s.setSort('az')
    const { askService } = await import('@/services')
    vi.spyOn(askService, 'ask').mockRejectedValue(new Error('down'))
    await s.submitAsk('x')
    expect(s.answerActive).toBe(true)
    expect(s.sort).toBe('relevance')
    s.clearThread()
    expect(s.sort).toBe('az')
  })

  it('refine keeps an explicitly chosen sort', async () => {
    const s = useCatalogueStore()
    s.catalogue = [t({ id: 1, score: 2 }), t({ id: 2, score: 8 })]
    s.applyResult('q', { ids: [1, 2], line: '', sub: '' })
    s.setSort('rating')
    await s.refine('lighter')
    expect(s.sort).toBe('rating')
    s.clearThread()
    expect(s.sort).toBe('trending') // browse sort from before the answer
  })

  it('stepThread leaves sort alone', () => {
    const s = useCatalogueStore()
    s.catalogue = [t({ id: 1 }), t({ id: 2 })]
    s.applyResult('a', { ids: [1], line: '', sub: '' })
    s.applyResult('b', { ids: [2], line: '', sub: '' })
    s.setSort('az')
    s.stepThread(0)
    expect(s.sort).toBe('az')
  })
})

describe('for you', () => {
  const cat = () => [
    t({ id: 1, score: 1 }),
    t({ id: 2, score: 2 }),
    t({ id: 3, score: 3 }),
    t({ id: 4, score: 4 }),
  ]
  const ready = (s: ReturnType<typeof useCatalogueStore>, fy: number[], basis = 4) => {
    s.forYou = { status: 'ready', ids: fy, basis }
  }

  it('ids order first, then the rest in trending order', () => {
    const s = useCatalogueStore(); s.catalogue = cat(); ready(s, [2, 1])
    s.setSort('foryou')
    expect(ids(s)).toEqual([2, 1, 4, 3])
  })

  it('sinks locally watched or rated titles even if in ids', async () => {
    const s = useCatalogueStore(); s.catalogue = cat(); ready(s, [2, 1, 3])
    s.setSort('foryou')
    s.ratings[1] = 8
    expect(ids(s)).toEqual([2, 3, 4, 1])
    vi.mocked(setWatched).mockReturnValue(new Promise(() => {})) // optimistic only
    void s.toggleWatched(2)
    expect(ids(s)).toEqual([3, 4, 2, 1])
  })

  it('with empty ids renders everything sunk', () => {
    const s = useCatalogueStore(); s.catalogue = cat(); ready(s, [], 4)
    s.setSort('foryou')
    expect(s.forYouAvailable).toBe(true)
    expect(ids(s)).toEqual([4, 3, 2, 1])
  })

  it('renders trending order until the first load is ready', () => {
    const s = useCatalogueStore(); s.catalogue = cat()
    s.forYou = { status: 'loading', ids: [1], basis: 4 }
    s.setSort('foryou')
    expect(ids(s)).toEqual([4, 3, 2, 1])
    s.forYou = { status: 'error', ids: [1], basis: 4 }
    expect(ids(s)).toEqual([4, 3, 2, 1])
  })

  it('a reload keeps the previous ids until the new response arrives', async () => {
    const s = useCatalogueStore(); s.catalogue = cat(); ready(s, [1])
    s.setSort('foryou')
    const d = deferred<ForYouResult>()
    vi.mocked(getForYou).mockReturnValue(d.promise)
    const p = s.loadForYou()
    expect(s.forYou.status).toBe('ready')
    expect(s.forYou.ids).toEqual([1])
    expect(ids(s)[0]).toBe(1)
    d.resolve({ ids: [3], basis: 5 })
    await p
    expect(s.forYou).toEqual({ status: 'ready', ids: [3], basis: 5 })
  })

  it('first load sets loading, and failure sets error but keeps ids', async () => {
    const s = useCatalogueStore()
    const d = deferred<ForYouResult>()
    vi.mocked(getForYou).mockReturnValue(d.promise)
    const p = s.loadForYou()
    expect(s.forYou.status).toBe('loading')
    d.resolve({ ids: [1], basis: 3 })
    await p
    vi.mocked(getForYou).mockRejectedValue(new Error('nope'))
    await s.loadForYou()
    expect(s.forYou.status).toBe('error')
    expect(s.forYou.ids).toEqual([1])
  })

  it('a superseded for-you response is ignored', async () => {
    const s = useCatalogueStore()
    const d1 = deferred<ForYouResult>()
    const d2 = deferred<ForYouResult>()
    vi.mocked(getForYou).mockReturnValueOnce(d1.promise).mockReturnValueOnce(d2.promise)
    const p1 = s.loadForYou()
    const p2 = s.loadForYou()
    d2.resolve({ ids: [2], basis: 4 })
    await p2
    d1.resolve({ ids: [1], basis: 9 })
    await p1
    expect(s.forYou).toEqual({ status: 'ready', ids: [2], basis: 4 })
  })

  it('a superseded failure does not clobber the newer result', async () => {
    const s = useCatalogueStore()
    const d1 = deferred<ForYouResult>()
    const d2 = deferred<ForYouResult>()
    vi.mocked(getForYou).mockReturnValueOnce(d1.promise).mockReturnValueOnce(d2.promise)
    const p1 = s.loadForYou()
    const p2 = s.loadForYou()
    d2.resolve({ ids: [2], basis: 4 })
    await p2
    d1.reject(new Error('late'))
    await p1
    expect(s.forYou.status).toBe('ready')
    expect(s.forYou.ids).toEqual([2])
  })

  it('forYouAvailable is false below basis 3', () => {
    const s = useCatalogueStore()
    s.forYou = { status: 'ready', ids: [], basis: 2 }
    expect(s.forYouAvailable).toBe(false)
    s.forYou = { status: 'ready', ids: [], basis: 3 }
    expect(s.forYouAvailable).toBe(true)
  })

  it('availability and the unlock lock follow the known basis, not the load status', async () => {
    const s = useCatalogueStore()
    // idle / in flight: unavailable, but not "locked" (basis unknown)
    expect(s.forYouAvailable).toBe(false)
    expect(s.forYouLocked).toBe(false)
    const d = deferred<ForYouResult>()
    vi.mocked(getForYou).mockReturnValueOnce(d.promise)
    const p = s.loadForYou()
    expect(s.forYou.status).toBe('loading')
    expect(s.forYouAvailable).toBe(false)
    expect(s.forYouLocked).toBe(false)
    // failed first fetch: unavailable, not locked
    d.reject(new Error('down'))
    await p
    expect(s.forYou.status).toBe('error')
    expect(s.forYouAvailable).toBe(false)
    expect(s.forYouLocked).toBe(false)
    // ready below the threshold: locked
    vi.mocked(getForYou).mockResolvedValueOnce({ ids: [], basis: 2 })
    await s.loadForYou()
    expect(s.forYouAvailable).toBe(false)
    expect(s.forYouLocked).toBe(true)
    // ready at the threshold, then a failed reload keeps the last good basis
    vi.mocked(getForYou).mockResolvedValueOnce({ ids: [1], basis: 3 })
    await s.loadForYou()
    expect(s.forYouAvailable).toBe(true)
    vi.mocked(getForYou).mockRejectedValueOnce(new Error('down'))
    await s.loadForYou()
    expect(s.forYou.status).toBe('error')
    expect(s.forYouAvailable).toBe(true)
    expect(s.forYouLocked).toBe(false)
  })

  it('falls back to the browse sort when For you becomes unavailable', async () => {
    const s = useCatalogueStore(); s.catalogue = cat(); ready(s, [1], 4)
    s.browseSort = 'rating'
    s.setSort('foryou')
    vi.mocked(getForYou).mockResolvedValueOnce({ ids: [], basis: 2 })
    await s.loadForYou()
    expect(s.sort).toBe('rating')
  })

  it('falls back to trending when the browse sort is itself For you', async () => {
    const s = useCatalogueStore(); s.catalogue = cat(); ready(s, [1], 4)
    s.browseSort = 'foryou'
    s.setSort('foryou')
    vi.mocked(getForYou).mockResolvedValueOnce({ ids: [], basis: 1 })
    await s.loadForYou()
    expect(s.sort).toBe('trending')
  })

  it('keeps another sort untouched when For you becomes unavailable', async () => {
    const s = useCatalogueStore(); s.catalogue = cat(); ready(s, [1], 4)
    s.setSort('az')
    vi.mocked(getForYou).mockResolvedValueOnce({ ids: [], basis: 0 })
    await s.loadForYou()
    expect(s.sort).toBe('az')
  })
})

describe('for-you refresh triggers', () => {
  it('runs after load(), setRating, clearRating and toggleWatched succeed', async () => {
    const s = useCatalogueStore()
    vi.mocked(getCatalogue).mockResolvedValue([t({ id: 1 })])
    await s.load()
    expect(getForYou).toHaveBeenCalledTimes(1)
    vi.mocked(setRating).mockResolvedValue({ rating: 7 })
    await s.setRating(1, 7)
    expect(getForYou).toHaveBeenCalledTimes(2)
    vi.mocked(clearRating).mockResolvedValue({ rating: null })
    await s.clearRating(1)
    expect(getForYou).toHaveBeenCalledTimes(3)
    vi.mocked(setWatched).mockResolvedValue({ watched: true })
    await s.toggleWatched(1)
    expect(getForYou).toHaveBeenCalledTimes(4)
  })

  it('does not run when the write fails', async () => {
    const s = useCatalogueStore()
    vi.mocked(setRating).mockRejectedValue(new Error('x'))
    await s.setRating(1, 5)
    vi.mocked(clearRating).mockRejectedValue(new Error('x'))
    await s.clearRating(1)
    vi.mocked(setWatched).mockRejectedValue(new Error('x'))
    await s.toggleWatched(1)
    expect(getForYou).not.toHaveBeenCalled()
  })

  it('a loadForYou failure never throws out of the actions', async () => {
    const s = useCatalogueStore()
    vi.mocked(getForYou).mockRejectedValue(new Error('down'))
    vi.mocked(setRating).mockResolvedValue({ rating: 7 })
    await expect(s.setRating(1, 7)).resolves.toBeUndefined()
    await vi.waitFor(() => expect(s.forYou.status).toBe('error'))
    expect(s.userDataError).toBeNull()
  })
})
