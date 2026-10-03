import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { getForYou } from '@/api/forYou'

function mockFetch(status: number, body: unknown) {
  return vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
  } as Response)
}

beforeEach(() => { vi.restoreAllMocks() })
afterEach(() => { vi.unstubAllGlobals() })

describe('getForYou', () => {
  it('resolves ids and basis from /api/for-you', async () => {
    const f = mockFetch(200, { ids: [3, 1], basis: 4 })
    vi.stubGlobal('fetch', f)
    await expect(getForYou()).resolves.toEqual({ ids: [3, 1], basis: 4 })
    expect(f).toHaveBeenCalledWith('/api/for-you')
  })

  it('rejects on HTTP 500', async () => {
    vi.stubGlobal('fetch', mockFetch(500, null))
    await expect(getForYou()).rejects.toThrow(/HTTP 500/)
  })

  it('rejects a non-array ids', async () => {
    vi.stubGlobal('fetch', mockFetch(200, { ids: 'x' }))
    await expect(getForYou()).rejects.toThrow(/invalid/i)
  })

  it('rejects non-numeric ids entries or a missing basis', async () => {
    vi.stubGlobal('fetch', mockFetch(200, { ids: [1, 'a'], basis: 3 }))
    await expect(getForYou()).rejects.toThrow(/invalid/i)
    vi.stubGlobal('fetch', mockFetch(200, { ids: [1] }))
    await expect(getForYou()).rejects.toThrow(/invalid/i)
  })
})
