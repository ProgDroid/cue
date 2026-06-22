import { describe, it, expect, vi, afterEach } from 'vitest'
import { getCatalogue } from '../client'

const sample = [{
  id: 1, imdbId: 'tt1', title: 'X', year: 2020, services: ['plex'],
  type: 'movie', genres: ['Comedy'], imdb: 8.1, len: '90 min',
  desc: 'd', cast: ['A'], watched: false, rating: null,
}]

afterEach(() => vi.restoreAllMocks())

describe('getCatalogue', () => {
  it('returns parsed titles on 200', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true, status: 200, json: async () => sample,
    }))
    const titles = await getCatalogue()
    expect(titles).toHaveLength(1)
    expect(titles[0].services).toEqual(['plex'])
  })

  it('throws on non-2xx', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 500, json: async () => ({}) }))
    await expect(getCatalogue()).rejects.toThrow(/catalogue/i)
  })
})
