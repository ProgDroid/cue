import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import StarRating from '../StarRating.vue'

describe('StarRating', () => {
  it('renders 5 stars and emits set with the clicked index (1-based)', async () => {
    const w = mount(StarRating, { props: { value: null } })
    const stars = w.findAll('[data-test="star"]')
    expect(stars).toHaveLength(5)
    await stars[3].trigger('click')
    expect(w.emitted('set')?.[0]).toEqual([4])
  })

  it('marks stars up to value as filled', () => {
    const w = mount(StarRating, { props: { value: 3 } })
    const filled = w.findAll('[data-test="star"].filled')
    expect(filled).toHaveLength(3)
  })
})
