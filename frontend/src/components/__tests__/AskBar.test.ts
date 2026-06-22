import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import AskBar from '../AskBar.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))

describe('AskBar', () => {
  it('submitting calls store.submitAsk with the input value', async () => {
    const w = mount(AskBar)
    const s = useCatalogueStore()
    const spy = vi.spyOn(s, 'submitAsk').mockResolvedValue()
    await w.get('[data-test="ask-input"]').setValue('cozy and low-stakes')
    await w.get('[data-test="ask-submit"]').trigger('click')
    expect(spy).toHaveBeenCalledWith('cozy and low-stakes')
  })
})
