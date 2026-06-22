import { describe, it, expect, vi, beforeEach } from 'vitest'
import { getCatalogue } from '../client'

const validTitle = {
  id: 1, imdbId: null, title: 'A', year: 2020, services: ['plex'], type: 'movie',
  genres: ['Drama'], imdb: 7, len: '100 min', desc: '', cast: [], watched: false, rating: null,
}

describe('getCatalogue', () => {
  beforeEach(() => { vi.restoreAllMocks() })

  it('returns validated titles on a well-formed response', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => [validTitle] }))
    const out = await getCatalogue()
    expect(out).toHaveLength(1)
    expect(out[0].id).toBe(1)
  })

  it('throws when the body is not an array', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) }))
    await expect(getCatalogue()).rejects.toThrow(/array|invalid/i)
  })

  it('throws when an item is missing required fields', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => [{ id: 1 }] }))
    await expect(getCatalogue()).rejects.toThrow(/invalid|title/i)
  })
})
