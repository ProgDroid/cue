import { describe, it, expect, vi, afterEach } from 'vitest'
import { triggerSync, getSyncStatus } from './sync'

afterEach(() => vi.restoreAllMocks())

describe('sync api', () => {
  it('triggerSync returns "started" on 202', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 202, ok: true }))
    expect(await triggerSync()).toBe('started')
  })

  it('triggerSync returns "running" on 409', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 409, ok: false }))
    expect(await triggerSync()).toBe('running')
  })

  it('getSyncStatus parses the status body', async () => {
    const body = {
      running: false,
      lastRun: { status: 'ok', itemCount: 3, finishedAt: '2026-06-22' },
      sources: [{ source: 'plex', lastRun: '2026-06-22', status: 'ok', itemCount: 3 }],
      catalogue: { titles: 3, movies: 2, series: 1, embedded: 3 },
    }
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(body) }))
    const s = await getSyncStatus()
    expect(s.catalogue.titles).toBe(3)
    expect(s.sources[0].source).toBe('plex')
  })
})
