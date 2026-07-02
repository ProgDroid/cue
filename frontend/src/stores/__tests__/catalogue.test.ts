import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { isReactive } from 'vue'
import { useCatalogueStore } from '../catalogue'
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
})
