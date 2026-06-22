import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import AppHeader from '../AppHeader.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))

describe('AppHeader', () => {
  it('typing in search updates store.query', async () => {
    const w = mount(AppHeader)
    const s = useCatalogueStore()
    await w.get('input[type="search"], input[data-test="search"]').setValue('frieren')
    expect(s.query).toBe('frieren')
  })

  it('renders the wordmark', () => {
    const w = mount(AppHeader)
    expect(w.text()).toContain('cue')
  })
})
