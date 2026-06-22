import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import { createPinia } from 'pinia'
import App from '../App.vue'
import { router } from '../router'

describe('App', () => {
  it('mounts without error', async () => {
    const wrapper = mount(App, { global: { plugins: [createPinia(), router] } })
    expect(wrapper.exists()).toBe(true)
  })
})
