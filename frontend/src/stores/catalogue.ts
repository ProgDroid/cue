import { defineStore } from 'pinia'
import { markRaw } from 'vue'
import type { ServiceKey, Title, TitleKind, ThreadStep, AskResult } from '@/types'
import { getCatalogue } from '@/api/client'
import { getForYou } from '@/api/forYou'
import { askService } from '@/services'
import { setRating as apiSetRating, clearRating as apiClearRating, setWatched as apiSetWatched } from '@/api/userData'

/** Unified best-available external rating on a 0–10 scale, or null if none. */
export function externalRating(t: Title): number | null {
  if (t.score != null) return t.score
  if (t.anilistScore != null) return t.anilistScore / 10
  return null
}

type Status = 'idle' | 'loading' | 'ready' | 'error'
type ServiceFilter = 'all' | ServiceKey
type TypeFilter = 'all' | TitleKind
export type SortKey = 'relevance' | 'trending' | 'foryou' | 'rating' | 'year' | 'az'

/** Minimum number of positively-rated/watched titles before "For you" is offered. */
const FOR_YOU_MIN_BASIS = 3

/** Monotonic id of the latest loadForYou call; responses from older calls are dropped. */
let forYouSeq = 0

/** Trending order: recently-new titles first (newest first), then best external rating, then id. */
function byTrending(a: Title, b: Title): number {
  if ((a.newSince != null) !== (b.newSince != null)) return a.newSince != null ? -1 : 1
  if (a.newSince != null && b.newSince != null && a.newSince !== b.newSince) return b.newSince - a.newSince
  const ra = externalRating(a)
  const rb = externalRating(b)
  if (ra !== rb) {
    if (ra == null) return 1
    if (rb == null) return -1
    return rb - ra
  }
  return a.id - b.id
}

interface ForYouState {
  status: Status
  ids: number[]
  basis: number
}

interface State {
  catalogue: Title[]
  status: Status
  error: string | null
  query: string
  service: ServiceFilter
  type: TypeFilter
  genre: 'all' | string
  sort: SortKey
  /** The sort to restore when an answer is cleared. */
  browseSort: SortKey
  forYou: ForYouState
  minRating: number
  watched: Record<number, boolean>
  ratings: Record<number, number>
  answerActive: boolean
  resultIds: number[]
  line: string
  sub: string
  thread: ThreadStep[]
  resolving: boolean
  askError: string | null
  userDataError: string | null
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
    browseSort: 'trending',
    forYou: { status: 'idle', ids: [], basis: 0 },
    minRating: 0,
    watched: {},
    ratings: {},
    answerActive: false,
    resultIds: [],
    line: '',
    sub: '',
    thread: [],
    resolving: false,
    askError: null,
    userDataError: null,
  }),

  getters: {
    forYouAvailable: (state): boolean => state.forYou.basis >= FOR_YOU_MIN_BASIS,
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
          .sort((a, b) => b.shared - a.shared || (b.t.score ?? -Infinity) - (a.t.score ?? -Infinity))
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
      const base = state.answerActive
        ? state.resultIds.map(id => state.catalogue.find(t => t.id === id)).filter((t): t is Title => !!t)
        : state.catalogue.slice()
      let out = base
      const q = state.query.trim().toLowerCase()
      if (q) out = out.filter(t => t.title.toLowerCase().includes(q))
      if (state.service !== 'all') out = out.filter(t => t.services.includes(state.service as ServiceKey))
      if (state.type !== 'all') out = out.filter(t => t.type === state.type)
      if (state.genre !== 'all') out = out.filter(t => t.genres.includes(state.genre))
      if (state.minRating > 0)
        out = out.filter(t => {
          const r = externalRating(t)
          return r != null && r >= state.minRating
        })

      const byRating = (a: Title, b: Title) =>
        (externalRating(b) ?? -Infinity) - (externalRating(a) ?? -Infinity)
      switch (state.sort) {
        case 'az': out.sort((a, b) => a.title.localeCompare(b.title)); break
        case 'year': out.sort((a, b) => b.year - a.year); break
        case 'rating': out.sort(byRating); break
        case 'trending': out.sort(byTrending); break
        case 'foryou': {
          const rank = new Map<number, number>()
          if (state.forYou.status === 'ready') state.forYou.ids.forEach((id, i) => rank.set(id, i))
          const isRanked = (t: Title) => rank.has(t.id) && !state.watched[t.id] && state.ratings[t.id] == null
          const ranked = out.filter(isRanked).sort((a, b) => (rank.get(a.id) ?? 0) - (rank.get(b.id) ?? 0))
          const rest = out.filter(t => !isRanked(t)).sort(byTrending)
          out = ranked.concat(rest)
          break
        }
        case 'relevance': /* keep base order */ break
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
    setMinRating(n: number) { this.minRating = n },

    async toggleWatched(id: number) {
      this.userDataError = null
      const prev = !!this.watched[id]
      const next = !prev
      this.watched[id] = next // optimistic
      try {
        const r = await apiSetWatched(id, next)
        this.watched[id] = r.watched // reconcile to server truth
        void this.loadForYou()
      } catch (e) {
        this.watched[id] = prev // rollback
        this.userDataError = e instanceof Error ? e.message : 'Could not update watched state.'
      }
    },

    async setRating(id: number, n: number) {
      this.userDataError = null
      const prev = this.ratings[id]
      this.ratings[id] = n // optimistic
      try {
        const r = await apiSetRating(id, n)
        if (r.rating != null) this.ratings[id] = r.rating
        void this.loadForYou()
      } catch (e) {
        if (prev == null) delete this.ratings[id]
        else this.ratings[id] = prev
        this.userDataError = e instanceof Error ? e.message : 'Could not save rating.'
      }
    },

    async clearRating(id: number) {
      this.userDataError = null
      const prev = this.ratings[id]
      delete this.ratings[id] // optimistic
      try {
        await apiClearRating(id)
        void this.loadForYou()
      } catch (e) {
        if (prev != null) this.ratings[id] = prev
        this.userDataError = e instanceof Error ? e.message : 'Could not clear rating.'
      }
    },

    async load() {
      this.status = 'loading'
      this.error = null
      try {
        this.catalogue = markRaw(await getCatalogue())
        for (const t of this.catalogue) {
          if (t.watched) this.watched[t.id] = true
          if (t.rating != null) this.ratings[t.id] = t.rating
        }
        this.status = 'ready'
      } catch (e) {
        this.error = e instanceof Error ? e.message : 'Failed to load catalogue'
        this.status = 'error'
        return
      }
      void this.loadForYou()
    },

    /** Fetch the For-you ranking. Never throws; a newer call supersedes older in-flight ones. */
    async loadForYou() {
      const seq = ++forYouSeq
      if (this.forYou.status !== 'ready') this.forYou.status = 'loading'
      try {
        const r = await getForYou()
        if (seq !== forYouSeq) return
        this.forYou = { status: 'ready', ids: r.ids, basis: r.basis }
      } catch {
        if (seq !== forYouSeq) return
        this.forYou.status = 'error' // keep the previous ids
      }
    },

    enterAnswer() {
      if (!this.answerActive) {
        this.browseSort = this.sort
        this.sort = 'relevance'
      }
      this.answerActive = true
    },

    applyResult(label: string, r: AskResult) {
      this.enterAnswer()
      this.resultIds = r.ids
      this.line = r.line
      this.sub = r.sub
      this.thread.push({ label, line: r.line, sub: r.sub, ids: r.ids })
    },

    async submitAsk(q: string) {
      this.askError = null
      this.resolving = true
      try {
        this.applyResult(q, await askService.ask(q, this.catalogue))
      } catch (e) {
        this.askError = e instanceof Error ? e.message : 'Ask is unavailable right now.'
        this.enterAnswer()
      } finally {
        this.resolving = false
      }
    },

    async refine(kind: 'lighter' | 'shorter' | 'surprise') {
      const current = this.resultIds
        .map(id => this.catalogue.find(t => t.id === id))
        .filter((t): t is Title => t != null)
      this.askError = null
      this.resolving = true
      try {
        this.applyResult(`↻ ${kind}`, await askService.refine(kind, current))
      } catch (e) {
        this.askError = e instanceof Error ? e.message : 'Ask is unavailable right now.'
        this.enterAnswer()
      } finally {
        this.resolving = false
      }
    },

    async moreLike(title: Title) {
      this.askError = null
      this.resolving = true
      try {
        this.applyResult(`≈ ${title.title}`, await askService.similar(title, this.catalogue))
      } catch (e) {
        this.askError = e instanceof Error ? e.message : 'Ask is unavailable right now.'
        this.enterAnswer()
      } finally {
        this.resolving = false
      }
    },

    stepThread(i: number) {
      const step = this.thread[i]; if (!step) return
      this.thread = this.thread.slice(0, i + 1)
      this.resultIds = step.ids; this.line = step.line; this.sub = step.sub; this.answerActive = true
    },

    clearThread() {
      if (this.answerActive) this.sort = this.browseSort
      this.answerActive = false; this.resultIds = []; this.line = ''; this.sub = ''; this.thread = []; this.askError = null
    },
  },
})
