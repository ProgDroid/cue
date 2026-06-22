import type { AskResult, Title } from '@/types'

const LIGHT = new Set(['Comedy', 'Animation', 'Adventure', 'Romance', 'Musical'])

// "164 min" -> 164; "28 eps" -> eps*~24 so series sort sensibly among movies.
function lenMinutes(len: string): number {
  const n = parseInt(len, 10) || 0
  return /eps/i.test(len) ? n * 24 : n
}

export interface AskService {
  ask(query: string, base: Title[]): Promise<AskResult>
  refine(kind: 'lighter' | 'shorter' | 'surprise', current: Title[]): Promise<AskResult>
  similar(title: Title, all: Title[]): Promise<AskResult>
}

export class StubAskService implements AskService {
  async ask(query: string, base: Title[]): Promise<AskResult> {
    await Promise.resolve() // keep the await seam so the shimmer is exercised
    const q = query.trim().toLowerCase()
    const hits = base.filter(t =>
      t.title.toLowerCase().includes(q) ||
      t.genres.some(g => g.toLowerCase().includes(q)) ||
      t.desc.toLowerCase().includes(q))
    const ids = (hits.length ? hits : base).map(t => t.id)
    return { line: `Here's what fits "${query}".`, sub: `${ids.length} · refine or filter to narrow`, ids }
  }

  async refine(kind: 'lighter' | 'shorter' | 'surprise', current: Title[]): Promise<AskResult> {
    await Promise.resolve()
    if (kind === 'lighter') {
      const ids = current.filter(t => t.genres.some(g => LIGHT.has(g))).map(t => t.id)
      return { line: 'Lighter picks.', sub: `${ids.length} · refine or filter to narrow`, ids }
    }
    if (kind === 'shorter') {
      const ids = [...current].sort((a, b) => lenMinutes(a.len) - lenMinutes(b.len)).map(t => t.id)
      return { line: 'Shortest first.', sub: `${ids.length} · refine or filter to narrow`, ids }
    }
    const pool = current.filter(t => (t.imdb ?? 0) >= 8)
    const pick = pool.length ? [pool[0].id] : current.slice(0, 1).map(t => t.id) // deterministic for tests
    return { line: 'A surprise for you.', sub: `${pick.length} · refine or filter to narrow`, ids: pick }
  }

  async similar(title: Title, all: Title[]): Promise<AskResult> {
    await Promise.resolve() // keep the await seam consistent with ask/refine
    const g = new Set(title.genres)
    const ids = all
      .filter(t => t.id !== title.id && t.genres.some(x => g.has(x)))
      .map(t => ({ id: t.id, shared: t.genres.filter(x => g.has(x)).length, imdb: t.imdb ?? -Infinity }))
      .sort((a, b) => b.shared - a.shared || b.imdb - a.imdb)
      .map(x => x.id)
    return { line: `More like ${title.title}.`, sub: `${ids.length} · refine or filter to narrow`, ids }
  }
}
