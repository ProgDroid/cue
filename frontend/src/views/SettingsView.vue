<template>
  <section class="settings">
    <header class="settings__head">
      <RouterLink to="/" class="settings__back">← back</RouterLink>
      <h1>Library &amp; sync</h1>
    </header>

    <div class="settings__row">
      <button
        type="button"
        data-test="sync-now"
        class="btn"
        :disabled="busy"
        @click="onSync"
      >{{ busy ? 'Syncing…' : 'Sync now' }}</button>
      <span v-if="message" class="settings__msg">{{ message }}</span>
    </div>

    <div v-if="status" class="settings__grid">
      <div class="card">
        <h2>Last run</h2>
        <p>{{ status.lastRun ? `${status.lastRun.status} · ${status.lastRun.itemCount} items` : 'never' }}</p>
        <p class="muted">{{ status.lastRun?.finishedAt ?? '' }}</p>
      </div>

      <div class="card">
        <h2>Sources</h2>
        <ul>
          <li v-for="s in status.sources" :key="s.source">
            <strong>{{ s.source }}</strong> — {{ s.status }} ({{ s.itemCount }})
            <span class="muted">{{ s.lastRun ?? 'never' }}</span>
          </li>
          <li v-if="status.sources.length === 0" class="muted">no runs yet</li>
        </ul>
      </div>

      <div class="card">
        <h2>Catalogue</h2>
        <p data-test="stat-titles">{{ status.catalogue.titles }} titles</p>
        <p class="muted">{{ status.catalogue.movies }} movies · {{ status.catalogue.series }} series</p>
        <p class="muted">{{ status.catalogue.embedded }} embedded</p>
      </div>

      <div class="card">
        <h2>Import IMDb ratings</h2>
        <input
          type="file"
          accept=".csv"
          data-test="imdb-file"
          :disabled="importing"
          @change="onImportFile"
        />
        <p v-if="importMsg" class="muted" data-test="imdb-result">{{ importMsg }}</p>
        <p v-if="importError" class="settings__error">{{ importError }}</p>
      </div>
    </div>

    <p v-if="error" class="settings__error">{{ error }}</p>
  </section>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { RouterLink } from 'vue-router'
import { triggerSync, getSyncStatus } from '@/api/sync'
import type { SyncStatus } from '@/types'
import { importRatings } from '@/api/userData'
import { useCatalogueStore } from '@/stores/catalogue'

const status = ref<SyncStatus | null>(null)
const busy = ref(false)
const message = ref('')
const error = ref('')
let poll: ReturnType<typeof setInterval> | undefined
const importing = ref(false)
const importMsg = ref('')
const importError = ref('')

// "1 rating" / "2 ratings" — naive English pluralization for the import summary.
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`

async function refresh() {
  try {
    const wasRunning = busy.value
    status.value = await getSyncStatus()
    busy.value = status.value.running
    if (!status.value.running && poll) {
      clearInterval(poll)
      poll = undefined
    }
    // A sync just finished (running -> not running): reload the catalogue so
    // newly-synced titles and imported watched flags surface without a reload.
    // Store accessed lazily so non-Pinia test mounts are unaffected.
    if (wasRunning && !status.value.running) {
      await useCatalogueStore().load()
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'failed to load status'
  }
}

async function onSync() {
  if (busy.value) return
  message.value = ''
  error.value = ''
  busy.value = true
  try {
    const result = await triggerSync()
    message.value = result === 'running' ? 'a sync is already running' : 'sync started'
    if (!poll) poll = setInterval(refresh, 3000)
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'failed to start sync'
    busy.value = false
  }
}

async function onImportFile(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  importing.value = true
  importMsg.value = ''
  importError.value = ''
  try {
    const csv = await file.text()
    const r = await importRatings(csv)
    importMsg.value = `Imported ${plural(r.imported, 'rating')} · ${r.matched} in your library · ${plural(r.skipped, 'row')} skipped`
    // Re-fetch so imported ratings surface without a manual reload.
    // Store accessed lazily here (not at setup) so tests that mount without
    // Pinia are unaffected.
    await useCatalogueStore().load()
  } catch (err) {
    importError.value = err instanceof Error ? err.message : 'Import failed.'
  } finally {
    importing.value = false
    input.value = '' // allow re-selecting the same file
  }
}

onMounted(refresh)
onUnmounted(() => { if (poll) clearInterval(poll) })
</script>

<style scoped>
.settings { max-width: 760px; margin: 0 auto; padding: 28px 22px; color: var(--text-primary, #e9ebf0); }
.settings__head { display: flex; align-items: baseline; gap: 16px; margin-bottom: 22px; }
.settings__back { color: var(--text-muted, #aab0bb); text-decoration: none; font-size: 13px; }
.settings__row { display: flex; align-items: center; gap: 14px; margin-bottom: 22px; }
.settings__msg { color: var(--text-muted, #aab0bb); font-size: 13px; }
.settings__error { color: var(--text-danger, #f5a3a3); margin-top: 16px; }
.settings__grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 16px; }
.card { background: var(--surface-1, #15171c); border: 1px solid var(--border, rgba(255,255,255,0.08)); border-radius: var(--r-md, 8px); padding: 16px; }
.card h2 { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-faint, #5f6570); margin: 0 0 10px; }
.card ul { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 6px; font-size: 13px; }
.muted { color: var(--text-faint, #5f6570); font-size: 12px; }
.btn { height: 36px; padding: 0 16px; border-radius: var(--r-md, 8px); border: 1px solid var(--accent-focus, rgba(245,197,24,0.5)); background: var(--surface-1, #15171c); color: var(--text-primary, #e9ebf0); cursor: pointer; font-family: var(--font-ui); }
.btn:disabled { opacity: 0.6; cursor: default; }
</style>
