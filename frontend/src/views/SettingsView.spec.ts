import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SettingsView from './SettingsView.vue'
import { createPinia, setActivePinia } from 'pinia'
import { useCatalogueStore } from '@/stores/catalogue'
import * as userDataApi from '@/api/userData'

afterEach(() => vi.restoreAllMocks())

const status = {
  running: false,
  lastRun: { status: 'ok', itemCount: 3, finishedAt: '2026-06-22T03:00:00' },
  sources: [{ source: 'plex', lastRun: '2026-06-22T03:00:00', status: 'ok', itemCount: 3 }],
  catalogue: { titles: 3, movies: 2, series: 1, embedded: 3 },
}

describe('SettingsView', () => {
  it('renders catalogue stats from the status endpoint', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(status) }))
    const wrapper = mount(SettingsView, { global: { stubs: { RouterLink: true } } })
    await flushPromises()
    expect(wrapper.text()).toContain('plex')
    expect(wrapper.find('[data-test="stat-titles"]').text()).toContain('3')
  })

  it('triggers a sync when the button is clicked', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(status) }) // initial load
      .mockResolvedValueOnce({ status: 202, ok: true })                          // POST /api/sync
      .mockResolvedValue({ ok: true, json: () => Promise.resolve(status) })      // refresh
    vi.stubGlobal('fetch', fetchMock)
    const wrapper = mount(SettingsView, { global: { stubs: { RouterLink: true } } })
    await flushPromises()
    await wrapper.find('[data-test="sync-now"]').trigger('click')
    await flushPromises()
    expect(fetchMock).toHaveBeenCalledWith('/api/sync', { method: 'POST' })
  })

  it('reloads the catalogue when a sync transitions running -> done', async () => {
    const notRunningStatus = {
      running: false,
      lastRun: null,
      sources: [],
      catalogue: { titles: 0, movies: 0, series: 0, embedded: 0 },
    }
    const doneStatus = {
      running: false,
      lastRun: { status: 'ok', itemCount: 5, finishedAt: '2026-06-23T04:00:00' },
      sources: [],
      catalogue: { titles: 5, movies: 3, series: 2, embedded: 5 },
    }

    // onMounted: not running (busy=false). Click Sync → onSync sets busy=true,
    // POSTs, then calls refresh(). refresh() sees wasRunning=true, status=done
    // (running:false) → triggers load().
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(notRunningStatus) }) // onMounted GET
      .mockResolvedValueOnce({ status: 202, ok: true })                                    // POST /api/sync
      .mockResolvedValueOnce({ ok: true, json: () => Promise.resolve(doneStatus) })        // refresh GET
    vi.stubGlobal('fetch', fetchMock)

    const pinia = createPinia()
    setActivePinia(pinia)
    const loadSpy = vi.spyOn(useCatalogueStore(), 'load').mockResolvedValue()

    const wrapper = mount(SettingsView, {
      global: { plugins: [pinia], stubs: { RouterLink: true } },
    })
    await flushPromises() // onMounted: not running → busy=false, no reload
    expect(loadSpy).not.toHaveBeenCalled()

    await wrapper.find('[data-test="sync-now"]').trigger('click')
    await flushPromises() // onSync: busy=true → POST → refresh(wasRunning=true, done) → load()
    expect(loadSpy).toHaveBeenCalledTimes(1)
  })

  it('imports a ratings file and re-fetches the catalogue', async () => {
    // Initial status load for onMounted.
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(status) }))
    const importSpy = vi
      .spyOn(userDataApi, 'importRatings')
      .mockResolvedValue({ imported: 5, skipped: 1, matched: 3 })

    const pinia = createPinia()
    setActivePinia(pinia)
    // Spy on the same store instance the component will resolve (same pinia).
    const loadSpy = vi.spyOn(useCatalogueStore(), 'load').mockResolvedValue()

    const wrapper = mount(SettingsView, {
      global: { plugins: [pinia], stubs: { RouterLink: true } },
    })
    await flushPromises()

    const input = wrapper.find('[data-test="imdb-file"]')
    const file = new File(['Const,Your Rating\ntt1,9\n'], 'ratings.csv', { type: 'text/csv' })
    // jsdom's File lacks .text(); stub it.
    Object.defineProperty(file, 'text', { value: () => Promise.resolve('Const,Your Rating\ntt1,9\n') })
    Object.defineProperty(input.element, 'files', { value: [file], configurable: true })
    await input.trigger('change')
    await flushPromises()

    expect(importSpy).toHaveBeenCalledWith('Const,Your Rating\ntt1,9\n')
    expect(wrapper.find('[data-test="imdb-result"]').text()).toContain('Imported 5')
    expect(loadSpy).toHaveBeenCalled()
  })
})
