import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '../catalogue'
import type { Title } from '@/types'

function t(p: Partial<Title>): Title {
  return { id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], score: null, anilistScore: null, len: '90 min',
    watched: false, rating: null, newSince: null, ...p }
}

beforeEach(() => setActivePinia(createPinia()))

describe('similar + user data', () => {
  const cat: Title[] = [
    t({ id: 1, genres: ['Animation', 'Adventure'], score: 9 }),
    t({ id: 2, genres: ['Animation', 'Adventure'], score: 8 }), // 2 shared
    t({ id: 3, genres: ['Animation'], score: 8.5 }),            // 1 shared
    t({ id: 4, genres: ['Horror'], score: 9.9 }),               // 0 shared
  ]

  it('ranks by shared-genre count then imdb desc, excludes self, top 5', () => {
    const s = useCatalogueStore(); s.catalogue = cat
    expect(s.similar(1).map(x => x.id)).toEqual([2, 3])
  })

  it('toggleWatched and setRating update derived getters', () => {
    const s = useCatalogueStore(); s.catalogue = cat
    s.toggleWatched(1); expect(s.isWatched(1)).toBe(true)
    s.setRating(1, 4); expect(s.ratingOf(1)).toBe(4)
  })
})
