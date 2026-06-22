export type ServiceKey = 'plex' | 'disney' | 'crunchyroll'
export type TitleKind = 'movie' | 'series'

export interface Title {
  id: number
  imdbId: string | null
  title: string
  year: number
  services: ServiceKey[]
  type: TitleKind
  genres: string[]
  imdb: number | null
  len: string
  desc: string
  cast: string[]
  watched: boolean
  rating: number | null
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
