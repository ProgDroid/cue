function errMsg(status: number): string {
  if (status === 422) return 'No IMDb match — can\'t save for this title.'
  return `Request failed (HTTP ${status})`
}

export async function setRating(id: number, rating: number): Promise<{ rating: number | null }> {
  const res = await fetch(`/api/titles/${id}/rating`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ rating }),
  })
  if (!res.ok) throw new Error(errMsg(res.status))
  return (await res.json()) as { rating: number | null }
}

export async function clearRating(id: number): Promise<{ rating: null }> {
  const res = await fetch(`/api/titles/${id}/rating`, { method: 'DELETE' })
  if (!res.ok) throw new Error(errMsg(res.status))
  return (await res.json()) as { rating: null }
}

export async function setWatched(id: number, watched: boolean): Promise<{ watched: boolean }> {
  const res = await fetch(`/api/titles/${id}/watched`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ watched }),
  })
  if (!res.ok) throw new Error(errMsg(res.status))
  return (await res.json()) as { watched: boolean }
}

export async function importRatings(
  csv: string,
): Promise<{ imported: number; skipped: number; matched: number }> {
  const res = await fetch('/api/import/ratings', {
    method: 'POST',
    headers: { 'Content-Type': 'text/csv' },
    body: csv,
  })
  if (!res.ok) throw new Error(errMsg(res.status))
  return (await res.json()) as { imported: number; skipped: number; matched: number }
}
