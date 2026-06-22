import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import AnswerContext from '../AnswerContext.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))

describe('AnswerContext', () => {
  it('renders the answer line and refine chips trigger store.refine', async () => {
    const s = useCatalogueStore()
    s.answerActive = true; s.line = 'Lighter picks.'; s.sub = '3 · refine'
    const spy = vi.spyOn(s, 'refine').mockResolvedValue()
    const w = mount(AnswerContext)
    expect(w.text()).toContain('Lighter picks.')
    await w.get('[data-test="refine-lighter"]').trigger('click')
    expect(spy).toHaveBeenCalledWith('lighter')
  })
})
