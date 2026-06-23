import type { Title, TitleDetail } from '@/types'

function isTitle(v: unknown): v is Title {
  if (typeof v !== 'object' || v === null) return false
  const r = v as Record<string, unknown>
  return typeof r.id === 'number'
    && typeof r.title === 'string'
    && typeof r.year === 'number'
    && (r.type === 'movie' || r.type === 'series')
    && Array.isArray(r.services)
    && Array.isArray(r.genres)
    && typeof r.len === 'string'
    && typeof r.watched === 'boolean'
    // imdbId drives canRate in DetailView; imdb/rating render. Validate all three
    // (each nullable) so a bad value can't silently disable rating or mis-render.
    && (r.imdbId === null || typeof r.imdbId === 'string')
    && (r.imdb === null || typeof r.imdb === 'number')
    && (r.rating === null || typeof r.rating === 'number')
}

function isTitleDetail(v: unknown): v is TitleDetail {
  if (!isTitle(v)) return false
  const r = v as unknown as Record<string, unknown>
  return typeof r.desc === 'string' && Array.isArray(r.cast)
}

export class NotFoundError extends Error {}

export async function getCatalogue(): Promise<Title[]> {
  const res = await fetch('/api/catalogue')
  if (!res.ok) {
    throw new Error(`Failed to load catalogue (HTTP ${res.status})`)
  }
  const data: unknown = await res.json()
  if (!Array.isArray(data)) {
    throw new Error('catalogue response is not an array')
  }
  if (!data.every(isTitle)) {
    throw new Error('catalogue response contains an invalid title')
  }
  return data
}

export async function getTitle(id: number): Promise<TitleDetail> {
  const res = await fetch(`/api/titles/${id}`)
  if (res.status === 404) {
    throw new NotFoundError(`Title ${id} not found`)
  }
  if (!res.ok) {
    throw new Error(`Failed to load title (HTTP ${res.status})`)
  }
  const data: unknown = await res.json()
  if (!isTitleDetail(data)) {
    throw new Error('title detail response is invalid')
  }
  return data
}
