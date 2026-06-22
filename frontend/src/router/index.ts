import { createRouter, createWebHistory } from 'vue-router'
import BrowseView from '@/views/BrowseView.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', component: BrowseView },
  ],
})
