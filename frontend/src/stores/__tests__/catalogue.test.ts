import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { isReactive } from 'vue'
import { useCatalogueStore, externalRating } from '../catalogue'
import type { Title } from '@/types'

function t(p: Partial<Title>): Title {
  return {
    id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], score: null, anilistScore: null, len: '90 min',
    watched: false, rating: null, ...p,
  }
}

const fixtures: Title[] = [
  t({ id: 1, title: 'Frieren', type: 'series', services: ['crunchyroll'], genres: ['Animation', 'Adventure'], score: 9.0, year: 2023 }),
  t({ id: 2, title: 'Coco', type: 'movie', services: ['disney'], genres: ['Animation', 'Musical'], score: 8.4, year: 2017 }),
  t({ id: 3, title: 'Alien', type: 'movie', services: ['plex', 'disney'], genres: ['Horror'], score: 8.5, year: 1979 }),
]

beforeEach(() => {
  setActivePinia(createPinia())
  vi.restoreAllMocks()
})

describe('catalogue store', () => {
  it('load() populates catalogue and sets status ready', async () => {
    const s = useCatalogueStore()
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue(fixtures)
    await s.load()
    expect(s.status).toBe('ready')
    expect(s.catalogue).toHaveLength(3)
  })

  it('visibleTitles filters by service membership (array includes)', async () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures
    s.setService('disney')
    expect(s.visibleTitles.map(x => x.id).sort()).toEqual([2, 3])
  })

  it('visibleTitles filters by type and query substring (case-insensitive)', () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures
    s.setType('movie'); s.setQuery('ali')
    expect(s.visibleTitles.map(x => x.id)).toEqual([3])
  })

  it('sort=az orders by title; sort=year orders desc; sort=rating orders desc with nulls last', () => {
    const s = useCatalogueStore()
    s.catalogue = [...fixtures, t({ id: 4, title: 'Aaa', score: null, year: 1990 })]
    s.setSort('az')
    expect(s.visibleTitles.map(x => x.title)[0]).toBe('Aaa')
    s.setSort('year')
    expect(s.visibleTitles.map(x => x.id)[0]).toBe(1) // 2023
    s.setSort('rating')
    expect(s.visibleTitles.map(x => x.id).at(-1)).toBe(4) // null imdb last
  })

  it('sort=rating ranks an AniList-only title by its normalised score, not last', () => {
    const s = useCatalogueStore()
    s.catalogue = [
      t({ id: 1, title: 'Movie6', score: 6.0, anilistScore: null }),
      t({ id: 2, title: 'AnimeAL90', score: null, anilistScore: 90 }), // → 9.0
    ]
    s.setSort('rating')
    expect(s.visibleTitles.map(x => x.id)).toEqual([2, 1]) // 9.0 before 6.0
  })

  it('genres getter returns unique sorted genres', () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures
    expect(s.genres).toEqual(['Adventure', 'Animation', 'Horror', 'Musical'])
  })

  it('load() stores the catalogue non-reactively (markRaw)', async () => {
    const s = useCatalogueStore()
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue(fixtures)
    await s.load()
    expect(isReactive(s.catalogue)).toBe(false)
  })

  it('load() still seeds watched/ratings from title fields after markRaw', async () => {
    const seeded = [t({ id: 5, imdbId: 'tt5', watched: true, rating: 7 })]
    const s = useCatalogueStore()
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue(seeded)
    await s.load()
    expect(s.watched[5]).toBe(true)
    expect(s.ratings[5]).toBe(7)
  })

  it('externalRating prefers score, falls back to anilistScore/10, else null', () => {
    expect(externalRating(t({ score: 8.5, anilistScore: 70 }))).toBe(8.5)
    expect(externalRating(t({ score: null, anilistScore: 85 }))).toBe(8.5)
    expect(externalRating(t({ score: null, anilistScore: null }))).toBeNull()
  })

  it('minRating=0 (default) applies no rating filter', () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures
    expect(s.visibleTitles).toHaveLength(3)
  })

  it('minRating hides titles with no external rating', () => {
    const s = useCatalogueStore()
    s.catalogue = [...fixtures, t({ id: 4, title: 'Unrated', score: null, anilistScore: null })]
    s.setMinRating(8)
    expect(s.visibleTitles.map(x => x.id).sort()).toEqual([1, 2, 3]) // id 4 dropped
  })

  it('minRating compares AniList titles on the normalised 0-10 scale', () => {
    const s = useCatalogueStore()
    s.catalogue = [t({ id: 10, title: 'AL85', score: null, anilistScore: 85 })]
    s.setMinRating(8)
    expect(s.visibleTitles.map(x => x.id)).toEqual([10]) // 8.5 >= 8
    s.setMinRating(9)
    expect(s.visibleTitles).toHaveLength(0) // 8.5 < 9
  })

  it('minRating composes with the service filter', () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures // 1:9.0 crunchyroll, 2:8.4 disney, 3:8.5 plex+disney
    s.setService('disney')
    s.setMinRating(8.5) // note: values in UI are whole, but getter must handle the threshold
    expect(s.visibleTitles.map(x => x.id).sort()).toEqual([3]) // 2 is 8.4 < 8.5, 1 not disney
  })

  it('minRating narrows an active Ask result', () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures
    s.applyResult('q', { line: '', sub: '', ids: [1, 2, 3] })
    s.setMinRating(9)
    expect(s.visibleTitles.map(x => x.id)).toEqual([1]) // only the 9.0
  })
})
