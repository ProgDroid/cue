import { describe, it, expect } from 'vitest'
import { StubAskService } from '../askService'
import type { Title } from '@/types'

function t(p: Partial<Title>): Title {
  return { id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], imdb: null, len: '90 min',
    watched: false, rating: null, ...p }
}
const cat: Title[] = [
  t({ id: 1, title: 'Frieren', genres: ['Animation', 'Adventure'], imdb: 9.0, len: '28 eps' }),
  t({ id: 2, title: 'Coco', genres: ['Animation', 'Musical'], imdb: 8.4, len: '105 min' }),
  t({ id: 3, title: 'Alien', genres: ['Horror'], imdb: 8.5, len: '117 min' }),
]
const svc = new StubAskService()

describe('StubAskService', () => {
  it('ask() keyword-matches over title/genre and returns ids + a line', async () => {
    const r = await svc.ask('animation', cat)
    expect(r.ids).toEqual(expect.arrayContaining([1, 2]))
    expect(r.ids).not.toContain(3)
    expect(r.line.length).toBeGreaterThan(0)
  })

  it("refine('lighter') keeps light genres", async () => {
    const r = await svc.refine('lighter', cat)
    expect(r.ids).toEqual(expect.arrayContaining([1, 2]))
    expect(r.ids).not.toContain(3)
  })

  it("refine('shorter') orders by ascending watch length", async () => {
    // lenMinutes: Coco 105 min -> 105, Alien 117 min -> 117, Frieren 28 eps -> 28*24=672
    const r = await svc.refine('shorter', cat)
    expect(r.ids).toEqual([2, 3, 1])
  })

  it('similar() ranks by shared genre', async () => {
    const r = await svc.similar(cat[0], cat)
    expect(r.ids).toEqual([2])
    expect(r.line).toContain('Frieren')
  })
})
