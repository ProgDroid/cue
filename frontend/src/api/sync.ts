import type { SyncStatus } from '@/types'

export async function triggerSync(): Promise<'started' | 'running'> {
  const res = await fetch('/api/sync', { method: 'POST' })
  if (res.status === 409) return 'running'
  if (res.status === 202) return 'started'
  throw new Error(`Failed to trigger sync (HTTP ${res.status})`)
}

export async function getSyncStatus(): Promise<SyncStatus> {
  const res = await fetch('/api/sync/status')
  if (!res.ok) throw new Error(`Failed to load sync status (HTTP ${res.status})`)
  return (await res.json()) as SyncStatus
}
