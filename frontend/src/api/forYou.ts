import type { ForYouResult } from '@/types'

export async function getForYou(): Promise<ForYouResult> {
  const res = await fetch('/api/for-you')
  if (!res.ok) {
    throw new Error(`Failed to load for-you (HTTP ${res.status})`)
  }
  const data: unknown = await res.json()
  if (typeof data !== 'object' || data === null) {
    throw new Error('for-you response is invalid')
  }
  const r = data as Record<string, unknown>
  if (!Array.isArray(r.ids) || !r.ids.every((i) => typeof i === 'number') || typeof r.basis !== 'number') {
    throw new Error('for-you response is invalid')
  }
  return { ids: r.ids as number[], basis: r.basis }
}
