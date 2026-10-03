import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import SettingsView from '../SettingsView.vue'
import * as sync from '@/api/sync'
import type { MotnStatus, SyncStatus } from '@/types'

vi.mock('@/api/sync')

const NOW_MS = Date.UTC(2026, 9, 3, 12, 0, 0) // 3 Oct 2026 12:00 UTC
const NOW = NOW_MS / 1000
const SEED_AT = Date.UTC(2026, 8, 12, 10, 0, 0) / 1000 // 12 Sep 2026 UTC

const motn: MotnStatus = {
  cacheSize: 4812,
  lastMode: 'delta',
  lastSeedAt: SEED_AT,
  seedFailedAt: null,
  catalogsCheckedAt: null,
  requestsThisMonth: 37,
  monthlyLimit: 500,
}

function statusWith(m: MotnStatus | null | undefined): SyncStatus {
  return {
    running: false,
    lastRun: null,
    sources: [],
    catalogue: { titles: 10, movies: 6, series: 4, embedded: 10 },
    motn: m as MotnStatus | null,
  }
}

async function mountView(m: MotnStatus | null | undefined) {
  vi.mocked(sync.getSyncStatus).mockResolvedValue(statusWith(m))
  const router = createRouter({ history: createMemoryHistory(), routes: [
    { path: '/', component: { template: '<div>home</div>' } },
    { path: '/settings', component: SettingsView },
  ] })
  router.push('/settings'); await router.isReady()
  const w = mount(SettingsView, { global: { plugins: [router] } })
  await flushPromises()
  return w
}

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW_MS)
})
afterEach(() => { vi.useRealTimers() })

describe('SettingsView MOTN line', () => {
  it('renders the MOTN line', async () => {
    const w = await mountView(motn)
    const line = w.find('[data-test="motn-line"]')
    expect(line.exists()).toBe(true)
    const text = line.text()
    expect(text).toContain('Cache 4,812 shows')
    expect(text).toContain('last full seed 12 Sep')
    expect(text).toContain('delta')
    expect(text).toContain('37 / 500 requests this month (approx.)')
    expect(text).not.toContain('seed paused')
  })

  it('shows seed paused until … during back-off', async () => {
    const w = await mountView({ ...motn, seedFailedAt: NOW - 3600 })
    // failed 1h ago + 3 days = 6 Oct
    expect(w.find('[data-test="motn-line"]').text()).toContain('seed paused until 6 Oct')
  })

  it('does not show seed paused once the back-off has elapsed', async () => {
    const w = await mountView({ ...motn, seedFailedAt: NOW - 3 * 86_400 - 60 })
    expect(w.find('[data-test="motn-line"]').text()).not.toContain('seed paused')
  })

  it('omits "last full seed" when lastSeedAt is null', async () => {
    const w = await mountView({ ...motn, lastSeedAt: null })
    const text = w.find('[data-test="motn-line"]').text()
    expect(text).toContain('Cache 4,812 shows')
    expect(text).not.toContain('last full seed')
  })

  it('hides the MOTN line when motn is null', async () => {
    const w = await mountView(null)
    expect(w.find('[data-test="stat-titles"]').exists()).toBe(true)
    expect(w.find('[data-test="motn-line"]').exists()).toBe(false)
  })

  it('hides the MOTN line when motn is absent (older server)', async () => {
    const w = await mountView(undefined)
    expect(w.find('[data-test="stat-titles"]').exists()).toBe(true)
    expect(w.find('[data-test="motn-line"]').exists()).toBe(false)
  })
})
