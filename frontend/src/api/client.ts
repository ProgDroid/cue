import type { Title } from '@/types'

export async function getCatalogue(): Promise<Title[]> {
  const res = await fetch('/api/catalogue')
  if (!res.ok) {
    throw new Error(`Failed to load catalogue (HTTP ${res.status})`)
  }
  return (await res.json()) as Title[]
}
