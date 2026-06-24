import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import WatchLinks from '../WatchLinks.vue'

describe('WatchLinks', () => {
  it('renders a branded link per watchable service with the right href', () => {
    const w = mount(WatchLinks, { props: { id: 42, watchable: ['plex', 'crunchyroll'] } })
    const plex = w.get('[data-test="watch-plex"]')
    expect(plex.attributes('href')).toBe('/api/titles/42/watch/plex')
    expect(plex.attributes('target')).toBe('_blank')
    expect(plex.attributes('rel')).toBe('noopener noreferrer')
    expect(w.get('[data-test="watch-crunchyroll"]').attributes('href')).toBe('/api/titles/42/watch/crunchyroll')
  })

  it('renders nothing when watchable is empty', () => {
    const w = mount(WatchLinks, { props: { id: 1, watchable: [] } })
    expect(w.find('[data-test="watch-links"]').exists()).toBe(false)
  })
})
