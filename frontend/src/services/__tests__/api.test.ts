import { describe, it, expect, vi, beforeEach } from 'vitest'
import { ApiAskService } from '../askService'
import type { Title } from '@/types'

const t = (id: number, title: string): Title => ({
  id, imdbId: null, title, year: 2020, services: ['plex'], type: 'movie',
  genres: ['Drama'], score: 7, anilistScore: null, len: '100 min', watched: false, rating: null,
})

describe('ApiAskService', () => {
  beforeEach(() => { vi.restoreAllMocks() })

  it('ask() posts the query and returns the parsed result', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ line: 'Here you go.', sub: '1 · narrow', ids: [2] }),
    })
    vi.stubGlobal('fetch', fetchMock)
    const svc = new ApiAskService()
    const r = await svc.ask('cozy', [t(1, 'A'), t(2, 'B')])
    expect(r.ids).toEqual([2])
    expect(fetchMock).toHaveBeenCalledWith('/api/ask', expect.objectContaining({ method: 'POST' }))
  })

  it('similar() posts anchorId and returns the parsed result', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ line: 'More like A.', sub: '1 · narrow', ids: [2] }),
    })
    vi.stubGlobal('fetch', fetchMock)
    const svc = new ApiAskService()
    const r = await svc.similar(t(1, 'A'), [t(1, 'A'), t(2, 'B')])
    expect(r.ids).toEqual([2])
    const [, opts] = fetchMock.mock.calls[0]
    expect(JSON.parse(opts.body)).toEqual({ anchorId: 1 })
  })

  it('throws a clear error on 503 so the store can surface "unavailable"', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 503 }))
    const svc = new ApiAskService()
    await expect(svc.ask('x', [])).rejects.toThrow(/unavailable|503/i)
  })

  it('rejects a malformed body (missing ids)', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true, json: async () => ({ line: 'x', sub: 'y' }),
    }))
    const svc = new ApiAskService()
    await expect(svc.ask('x', [])).rejects.toThrow(/ids/i)
  })
})
