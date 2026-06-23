import { describe, it, expect, vi, afterEach } from 'vitest'
import { importRatings } from '@/api/userData'

afterEach(() => vi.restoreAllMocks())

describe('importRatings', () => {
  it('POSTs the CSV as text/csv and returns the summary', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ imported: 2, skipped: 1, matched: 1 }),
    })
    vi.stubGlobal('fetch', fetchMock)

    const result = await importRatings('Const,Your Rating\ntt1,9\n')

    expect(fetchMock).toHaveBeenCalledWith('/api/import/ratings', {
      method: 'POST',
      headers: { 'Content-Type': 'text/csv' },
      body: 'Const,Your Rating\ntt1,9\n',
    })
    expect(result).toEqual({ imported: 2, skipped: 1, matched: 1 })
  })

  it('throws on a non-OK response', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 400 }))
    await expect(importRatings('garbage')).rejects.toThrow()
  })
})
