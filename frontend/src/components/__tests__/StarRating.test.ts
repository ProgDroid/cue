import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import StarRating from '../StarRating.vue'

describe('StarRating', () => {
  it('renders 10 pips and emits set with the clicked index (1-based)', async () => {
    const w = mount(StarRating, { props: { value: null } })
    const stars = w.findAll('[data-test="star"]')
    expect(stars).toHaveLength(10)
    await stars[6].trigger('click')
    expect(w.emitted('set')?.[0]).toEqual([7])
  })

  it('marks pips up to value as filled', () => {
    const w = mount(StarRating, { props: { value: 7 } })
    expect(w.findAll('[data-test="star"].filled')).toHaveLength(7)
  })

  it('clicking the current value emits clear instead of set', async () => {
    const w = mount(StarRating, { props: { value: 7 } })
    await w.findAll('[data-test="star"]')[6].trigger('click') // the 7th pip
    expect(w.emitted('clear')).toBeTruthy()
    expect(w.emitted('set')).toBeUndefined()
  })

  it('disabled blocks all emits', async () => {
    const w = mount(StarRating, { props: { value: null, disabled: true } })
    await w.findAll('[data-test="star"]')[3].trigger('click')
    expect(w.emitted('set')).toBeUndefined()
    expect(w.emitted('clear')).toBeUndefined()
  })

  it('exposes group + per-star labels and pressed state for screen readers', () => {
    const w = mount(StarRating, { props: { value: 7 } })
    const group = w.find('.star-row')
    expect(group.attributes('role')).toBe('group')
    expect(group.attributes('aria-label')).toBe('Your rating')
    const stars = w.findAll('[data-test="star"]')
    expect(stars[6].attributes('aria-label')).toBe('Rate 7 out of 10')
    expect(stars[6].attributes('aria-pressed')).toBe('true')
    expect(stars[5].attributes('aria-pressed')).toBe('false')
  })

  it('hovering a pip fills it and every pip before it', async () => {
    const w = mount(StarRating, { props: { value: null } })
    await w.findAll('[data-test="star"]')[3].trigger('mouseenter')
    const stars = w.findAll('[data-test="star"]')
    expect(w.findAll('[data-test="star"].filled')).toHaveLength(4)
    expect(stars[3].classes()).toContain('filled')
    expect(stars[4].classes()).not.toContain('filled')
  })

  it('hover previews over the saved value and reverts on leave', async () => {
    const w = mount(StarRating, { props: { value: 7 } })
    await w.findAll('[data-test="star"]')[2].trigger('mouseenter')
    expect(w.findAll('[data-test="star"].filled')).toHaveLength(3)
    await w.find('.star-row').trigger('mouseleave')
    expect(w.findAll('[data-test="star"].filled')).toHaveLength(7)
  })

  it('disabled ignores hover', async () => {
    const w = mount(StarRating, { props: { value: 2, disabled: true } })
    await w.findAll('[data-test="star"]')[8].trigger('mouseenter')
    expect(w.findAll('[data-test="star"].filled')).toHaveLength(2)
  })
})
