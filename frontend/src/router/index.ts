import { createRouter, createWebHistory } from 'vue-router'
import BrowseView from '@/views/BrowseView.vue'
import DetailView from '@/views/DetailView.vue'
import SettingsView from '@/views/SettingsView.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', component: BrowseView },
    { path: '/title/:id', component: DetailView },
    { path: '/settings', component: SettingsView },
  ],
})
