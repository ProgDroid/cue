import type { Title } from '@/types'

function isTitle(v: unknown): v is Title {
  if (typeof v !== 'object' || v === null) return false
  const r = v as Record<string, unknown>
  return typeof r.id === 'number'
    && typeof r.title === 'string'
    && typeof r.year === 'number'
    && (r.type === 'movie' || r.type === 'series')
    && Array.isArray(r.services)
    && Array.isArray(r.genres)
    && Array.isArray(r.cast)
    && typeof r.len === 'string'
    && typeof r.desc === 'string'
    && typeof r.watched === 'boolean'
}

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
