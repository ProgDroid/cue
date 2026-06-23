import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '@/stores/catalogue'

vi.mock('@/api/userData', () => ({
  setRating: vi.fn(),
  clearRating: vi.fn(),
  setWatched: vi.fn(),
}))
import { setRating, clearRating, setWatched } from '@/api/userData'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.clearAllMocks()
})

describe('catalogue store — user-data actions', () => {
  it('toggleWatched optimistically sets and reconciles on success', async () => {
    vi.mocked(setWatched).mockResolvedValue({ watched: true })
    const store = useCatalogueStore()
    await store.toggleWatched(5)
    expect(setWatched).toHaveBeenCalledWith(5, true)
    expect(store.isWatched(5)).toBe(true)
    expect(store.userDataError).toBeNull()
  })

  it('toggleWatched rolls back and records error on failure', async () => {
    vi.mocked(setWatched).mockRejectedValue(new Error('boom'))
    const store = useCatalogueStore()
    await store.toggleWatched(5)
    expect(store.isWatched(5)).toBe(false) // reverted
    expect(store.userDataError).toBe('boom')
  })

  it('setRating optimistically sets and rolls back on failure', async () => {
    vi.mocked(setRating).mockRejectedValue(new Error('nope'))
    const store = useCatalogueStore()
    store.ratings[5] = 3
    await store.setRating(5, 8)
    expect(store.ratingOf(5)).toBe(3) // rolled back to previous
    expect(store.userDataError).toBe('nope')
  })

  it('clearRating removes the rating on success', async () => {
    vi.mocked(clearRating).mockResolvedValue({ rating: null })
    const store = useCatalogueStore()
    store.ratings[5] = 9
    await store.clearRating(5)
    expect(store.ratingOf(5)).toBeNull()
    expect(clearRating).toHaveBeenCalledWith(5)
  })
})
