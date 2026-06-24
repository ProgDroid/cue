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
export interface SyncStatus {
  running: boolean
  lastRun: LastRun | null
  sources: SourceRun[]
  catalogue: CatalogueStats
}
