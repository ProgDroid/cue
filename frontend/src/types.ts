export type ServiceKey = 'plex' | 'disney' | 'crunchyroll'
export type TitleKind = 'movie' | 'series'

export interface TitleListItem {
  id: number
  imdbId: string | null
  title: string
  year: number
  services: ServiceKey[]
  type: TitleKind
  genres: string[]
  score: number | null
  anilistScore: number | null
  len: string
  watched: boolean
  rating: number | null
  /** Unix seconds when the title entered the catalogue within the "new" window; null otherwise. */
  newSince: number | null
}

/** The dominant in-store shape is the slim list item. */
export type Title = TitleListItem

export interface TitleDetail extends TitleListItem {
  desc: string
  cast: string[]
  watchable: ServiceKey[]
}

export interface AskResult {
  line: string
  sub: string
  ids: number[]
}

export interface ThreadStep {
  label: string
  line: string
  sub: string
  ids: number[]
}

export interface SourceRun {
  source: string
  lastRun: string | null
  status: string
  itemCount: number
}
export interface CatalogueStats {
  titles: number
  movies: number
  series: number
  embedded: number
}
export interface LastRun {
  status: string
  itemCount: number
  finishedAt: string | null
}
export interface MotnStatus {
  cacheSize: number
  lastMode: string | null
  lastSeedAt: number | null
  seedFailedAt: number | null
  catalogsCheckedAt: number | null
  requestsThisMonth: number
  monthlyLimit: number
}
export interface ForYouResult {
  ids: number[]
  basis: number
}
export interface SyncStatus {
  running: boolean
  lastRun: LastRun | null
  sources: SourceRun[]
  catalogue: CatalogueStats
  motn: MotnStatus | null
}
