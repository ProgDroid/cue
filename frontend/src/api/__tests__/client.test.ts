import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { getCatalogue, getTitle, NotFoundError } from '@/api/client'
import type { TitleDetail, ServiceKey } from '@/types'

const listItem = {
  id: 1, imdbId: 'tt1', title: 'Coco', year: 2017, services: ['disney'] as ServiceKey[],
  type: 'movie' as const, genres: ['Animation'], score: 8.4, anilistScore: null, len: '105 min',
  watched: false, rating: null,
}
const detail: TitleDetail = { ...listItem, desc: 'A boy.', cast: ['A. Gonzalez'] }

function mockFetch(status: number, body: unknown) {
  return vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
  } as Response)
}

beforeEach(() => { vi.restoreAllMocks() })
afterEach(() => { vi.unstubAllGlobals() })

describe('api client', () => {
  it('getCatalogue accepts slim list items (no desc/cast)', async () => {
    vi.stubGlobal('fetch', mockFetch(200, [listItem]))
    const out = await getCatalogue()
    expect(out).toHaveLength(1)
    expect(out[0].id).toBe(1)
  })

  it('getTitle returns the full detail', async () => {
    vi.stubGlobal('fetch', mockFetch(200, detail))
    const out = await getTitle(1)
    expect(out.desc).toBe('A boy.')
    expect(out.cast).toEqual(['A. Gonzalez'])
  })

  it('getTitle throws NotFoundError on 404', async () => {
    vi.stubGlobal('fetch', mockFetch(404, null))
    await expect(getTitle(9)).rejects.toBeInstanceOf(NotFoundError)
  })

  it('getTitle throws on malformed detail', async () => {
    vi.stubGlobal('fetch', mockFetch(200, { id: 1 }))
    await expect(getTitle(1)).rejects.toThrow(/invalid/)
  })

  it('getCatalogue throws when the body is not an array', async () => {
    vi.stubGlobal('fetch', mockFetch(200, {}))
    await expect(getCatalogue()).rejects.toThrow(/array|invalid/i)
  })

  it('getCatalogue throws when an item is missing required fields', async () => {
    vi.stubGlobal('fetch', mockFetch(200, [{ id: 1 }]))
    await expect(getCatalogue()).rejects.toThrow(/invalid|title/i)
  })

  it('getCatalogue rejects a wrong-typed imdbId (drives canRate)', async () => {
    vi.stubGlobal('fetch', mockFetch(200, [{ ...listItem, imdbId: 123 }]))
    await expect(getCatalogue()).rejects.toThrow(/invalid|title/i)
  })

  it('getCatalogue rejects a wrong-typed rating', async () => {
    vi.stubGlobal('fetch', mockFetch(200, [{ ...listItem, rating: 'high' }]))
    await expect(getCatalogue()).rejects.toThrow(/invalid|title/i)
  })

  it('getCatalogue throws on non-OK response', async () => {
    vi.stubGlobal('fetch', mockFetch(500, null))
    await expect(getCatalogue()).rejects.toThrow(/HTTP 500/)
  })

  it('getTitle throws a generic Error on HTTP 500 (not NotFoundError)', async () => {
    vi.stubGlobal('fetch', mockFetch(500, null))
    await expect(getTitle(1)).rejects.toThrow(/HTTP 500/)
    vi.stubGlobal('fetch', mockFetch(500, null))
    await expect(getTitle(1)).rejects.not.toBeInstanceOf(NotFoundError)
  })
})
