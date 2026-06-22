import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import PosterCard from '../PosterCard.vue'
import type { Title } from '@/types'

const title: Title = {
  id: 7, imdbId: 'tt7', title: 'Frieren', year: 2023, services: ['crunchyroll'],
  type: 'series', genres: ['Animation'], imdb: 9.0, len: '28 eps',
  desc: '', cast: [], watched: false, rating: null,
}

describe('PosterCard', () => {
  it('renders title and a monogram placeholder (no <img> until real art)', () => {
    const w = mount(PosterCard, { props: { title, watched: false } })
    expect(w.text()).toContain('Frieren')
    expect(w.find('img').exists()).toBe(false)
  })

  it('emits select on card click and find-similar on ✦', async () => {
    const w = mount(PosterCard, { props: { title, watched: false } })
    await w.get('[data-test="card"]').trigger('click')
    expect(w.emitted('select')?.[0]).toEqual([7])
    await w.get('[data-test="find-similar"]').trigger('click')
    expect(w.emitted('find-similar')?.[0]).toEqual([title])
  })

  it('shows the watched badge when watched', () => {
    const w = mount(PosterCard, { props: { title, watched: true } })
    expect(w.find('[data-test="watched-badge"]').exists()).toBe(true)
  })
})
