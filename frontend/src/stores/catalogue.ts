import { defineStore } from 'pinia'
import type { ServiceKey, Title, TitleKind } from '@/types'
import { getCatalogue } from '@/api/client'

type Status = 'idle' | 'loading' | 'ready' | 'error'
type ServiceFilter = 'all' | ServiceKey
type TypeFilter = 'all' | TitleKind
type SortKey = 'trending' | 'rating' | 'year' | 'az'

interface State {
  catalogue: Title[]
  status: Status
  error: string | null
  query: string
  service: ServiceFilter
  type: TypeFilter
  genre: 'all' | string
  sort: SortKey
  watched: Record<number, boolean>
  ratings: Record<number, number>
}

export const useCatalogueStore = defineStore('catalogue', {
  state: (): State => ({
    catalogue: [],
    status: 'idle',
    error: null,
    query: '',
    service: 'all',
    type: 'all',
    genre: 'all',
    sort: 'trending',
    watched: {},
    ratings: {},
  }),

  getters: {
    isWatched: (state) => (id: number): boolean => !!state.watched[id],
    ratingOf: (state) => (id: number): number | null => state.ratings[id] ?? null,
    similar() {
      return (id: number): Title[] => {
        const self = this.catalogue.find((t: Title) => t.id === id)
        if (!self) return []
        const selfGenres = new Set(self.genres)
        return (this.catalogue as Title[])
          .filter((t: Title) => t.id !== id && t.genres.some((g: string) => selfGenres.has(g)))
          .map((t: Title) => ({ t, shared: t.genres.filter((g: string) => selfGenres.has(g)).length }))
          .sort((a, b) => b.shared - a.shared || (b.t.imdb ?? -Infinity) - (a.t.imdb ?? -Infinity))
          .slice(0, 5)
          .map((x) => x.t)
      }
    },

    genres(state): string[] {
      const set = new Set<string>()
      for (const t of state.catalogue) for (const g of t.genres) set.add(g)
      return [...set].sort()
    },

    visibleTitles(state): Title[] {
      let out = state.catalogue.slice() // base set (answer set overrides this in Task 12)
      const q = state.query.trim().toLowerCase()
      if (q) out = out.filter(t => t.title.toLowerCase().includes(q))
      if (state.service !== 'all') out = out.filter(t => t.services.includes(state.service as ServiceKey))
      if (state.type !== 'all') out = out.filter(t => t.type === state.type)
      if (state.genre !== 'all') out = out.filter(t => t.genres.includes(state.genre))

      const byRating = (a: Title, b: Title) =>
        (b.imdb ?? -Infinity) - (a.imdb ?? -Infinity)
      switch (state.sort) {
        case 'az': out.sort((a, b) => a.title.localeCompare(b.title)); break
        case 'year': out.sort((a, b) => b.year - a.year); break
        case 'rating': out.sort(byRating); break
        case 'trending': /* keep base order */ break
      }
      return out
    },
  },

  actions: {
    setQuery(v: string) { this.query = v },
    setService(v: ServiceFilter) { this.service = v },
    setType(v: TypeFilter) { this.type = v },
    setGenre(v: 'all' | string) { this.genre = v },
    setSort(v: SortKey) { this.sort = v },

    toggleWatched(id: number) { this.watched[id] = !this.watched[id] },
    setRating(id: number, n: number) { this.ratings[id] = n },
    // Note: toggleWatched/setRating mutate local state only — persistence is Plan 5.

    async load() {
      this.status = 'loading'
      this.error = null
      try {
        this.catalogue = await getCatalogue()
        for (const t of this.catalogue) {
          if (t.watched) this.watched[t.id] = true
          if (t.rating != null) this.ratings[t.id] = t.rating
        }
        this.status = 'ready'
      } catch (e) {
        this.error = e instanceof Error ? e.message : 'Failed to load catalogue'
        this.status = 'error'
      }
    },
  },
})
