import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import FilterBar from '../FilterBar.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import type { Title } from '@/types'

beforeEach(() => setActivePinia(createPinia()))

describe('FilterBar', () => {
  it('clicking Plex sets store.service and marks the button active', async () => {
    const w = mount(FilterBar)
    const s = useCatalogueStore()
    await w.get('[data-test="service-plex"]').trigger('click')
    expect(s.service).toBe('plex')
    expect(w.get('[data-test="service-plex"]').classes()).toContain('active')
  })

  it('shows the result count from visibleTitles', () => {
    const s = useCatalogueStore()
    s.catalogue = [{ id: 1 } as Title, { id: 2 } as Title]
    const w = mount(FilterBar)
    expect(w.get('[data-test="count"]').text()).toMatch(/2 titles/)
  })
})
