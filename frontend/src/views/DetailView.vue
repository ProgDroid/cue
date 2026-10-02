<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useCatalogueStore } from '@/stores/catalogue'
import { getTitle, NotFoundError } from '@/api/client'
import { askService } from '@/services'
import type { Title, TitleDetail } from '@/types'
import ServicePill from '@/components/ServicePill.vue'
import StarRating from '@/components/StarRating.vue'
import WatchLinks from '@/components/WatchLinks.vue'
import { posterPlaceholder, monogram } from '@/design/tokens'

const route = useRoute()
const router = useRouter()
const store = useCatalogueStore()

const id = computed(() => Number(route.params.id))
const detail = ref<TitleDetail | null>(null)
const loading = ref(true)
// Distinguish a genuine 404 (title gone) from a transient failure (network/500)
// so the latter can offer a retry instead of a misleading "not found".
const loadError = ref<'notfound' | 'transient' | null>(null)

async function loadDetail(tid: number) {
  loading.value = true
  detail.value = null
  loadError.value = null
  try {
    detail.value = await getTitle(tid)
    loadSimilar(detail.value)
  } catch (e) {
    loadError.value = e instanceof NotFoundError ? 'notfound' : 'transient'
  } finally {
    loading.value = false
  }
}

function retry() { loadDetail(id.value) }

const ph = computed(() => detail.value ? posterPlaceholder(detail.value.title) : null)
const mono = computed(() => detail.value ? monogram(detail.value.title) : '')

const factLine = computed(() => {
  if (!detail.value) return ''
  const kind = detail.value.type === 'movie' ? 'Movie' : 'Series'
  return `${detail.value.year} · ${kind} · ${detail.value.len} · ${detail.value.genres.join(', ')}`
})

// Similar strip: server ranking (embedding cosine, genre-overlap fallback) via
// /api/ask/similar. `undefined` = in flight (render nothing, avoids a flash of
// the local list); `null` = request failed, use the local genre ranking.
const similarIds = ref<number[] | null | undefined>(undefined)
let similarSeq = 0

async function loadSimilar(anchor: Title) {
  const seq = ++similarSeq
  similarIds.value = undefined
  try {
    const { ids } = await askService.similar(anchor, store.catalogue)
    if (seq === similarSeq) similarIds.value = ids
  } catch {
    if (seq === similarSeq) similarIds.value = null
  }
}

const similar = computed((): Title[] => {
  if (!detail.value || similarIds.value === undefined) return []
  if (similarIds.value === null) return store.similar(id.value)
  const byId = new Map(store.catalogue.map((t) => [t.id, t]))
  return similarIds.value
    .map((sid) => byId.get(sid))
    .filter((t): t is Title => t !== undefined)
    .slice(0, 5)
})
// Precompute placeholder + monogram once per similar title (the template would
// otherwise call posterPlaceholder() three times per card on every render).
const simCards = computed(() =>
  similar.value.map((s) => ({
    id: s.id,
    title: s.title,
    ph: posterPlaceholder(s.title),
    mono: monogram(s.title),
  })),
)

function back() { router.push('/') }

const idIsWatched = computed(() => store.isWatched(id.value))
const idRating = computed(() => store.ratingOf(id.value))
const canRate = computed(() => !!detail.value?.imdbId)
function toggleWatched() { store.toggleWatched(id.value) }
function setRating(n: number) { store.setRating(id.value, n) }
function clearRating() { store.clearRating(id.value) }

onMounted(() => {
  loadDetail(id.value)
  if (store.catalogue.length === 0) store.load()
})
watch(id, (n) => loadDetail(n))

const posterFailed = ref(false)
const backdropFailed = ref(false)
const simFailed = ref<Record<number, boolean>>({})
</script>

<template>
  <div v-if="detail && ph" class="detail-root">
    <!-- Backdrop band -->
    <div class="backdrop" :style="{ background: ph.backdrop }">
      <!-- Motif blob -->
      <div class="backdrop-motif" :style="{ background: ph.motif }" />
      <!-- Large monogram -->
      <div class="backdrop-mono" :style="{ color: ph.glyphColor }">{{ mono }}</div>
      <!-- Real art overlay -->
      <img
        v-show="detail && !backdropFailed"
        :src="`/api/titles/${detail.id}/backdrop`"
        alt=""
        class="backdrop-img"
        @error="backdropFailed = true"
      />
      <!-- Bottom fade scrim -->
      <div class="scrim-bottom" />
      <!-- Left fade scrim -->
      <div class="scrim-left" />
      <!-- Back button -->
      <button class="back-btn" @click="back">← Library</button>
    </div>

    <!-- Body pulled up over backdrop -->
    <div class="body">
      <!-- Left: poster column -->
      <div class="poster-col">
        <div class="poster" :style="{ background: ph.background }">
          <div class="poster-motif" :style="{ background: ph.motif }" />
          <div class="poster-mono" :style="{ color: ph.glyphColor }">{{ mono }}</div>
          <img
            v-show="detail && !posterFailed"
            :src="`/api/titles/${detail.id}/poster`"
            loading="lazy"
            alt=""
            class="poster-img"
            @error="posterFailed = true"
          />
        </div>

        <!-- Mark as watched button -->
        <button
          data-test="mark-watched"
          class="watched-btn"
          :class="{ 'watched-btn--active': idIsWatched }"
          :disabled="!canRate"
          @click="toggleWatched"
        >
          {{ idIsWatched ? '✓ Watched' : 'Mark as watched' }}
        </button>

        <WatchLinks :id="detail.id" :watchable="detail.watchable" />

        <!-- Your rating well -->
        <div class="rating-well">
          <div class="rating-eyebrow">Your rating</div>
          <StarRating
            :value="idRating"
            :disabled="!canRate"
            @set="setRating"
            @clear="clearRating"
          />
          <p v-if="!canRate" class="no-imdb-hint">No IMDb match — can't save ratings.</p>
        </div>

        <!-- Write-failure feedback: rating/watched writes roll back optimistically
             on error; without this the change would silently revert. -->
        <p v-if="store.userDataError" class="userdata-error" data-test="userdata-error">
          {{ store.userDataError }}
        </p>
      </div>

      <!-- Right: info column -->
      <div class="info-col">
        <!-- Badge row: service pills + rating pills -->
        <div class="badge-row">
          <ServicePill
            v-for="svc in detail.services"
            :key="svc"
            :service="svc"
            class="svc-pill-wrap"
          />
          <span v-if="detail.anilistScore !== null" data-test="detail-anilist" class="rating-pill anilist-pill">
            <span class="pill-mark">AL</span>
            <span class="pill-score">{{ detail.anilistScore }}</span>
            <span class="pill-label">AniList</span>
          </span>
          <span v-if="detail.score !== null" data-test="detail-score" class="rating-pill">
            <span class="pill-mark">★</span>
            <span class="pill-score">{{ detail.score }}</span>
            <span class="pill-label">Rating</span>
          </span>
        </div>

        <!-- Title -->
        <h1 class="title-h1">{{ detail.title }}</h1>

        <!-- Fact line -->
        <div class="fact-line">{{ factLine }}</div>

        <!-- Description -->
        <p class="desc">{{ detail.desc }}</p>

        <!-- Cast -->
        <div class="cast-section">
          <div class="section-eyebrow">Cast</div>
          <div class="cast-chips">
            <span v-for="person in detail.cast" :key="person" class="cast-chip">{{ person }}</span>
          </div>
        </div>

        <!-- Similar titles mini-grid -->
        <div v-if="similar.length > 0" class="similar-section">
          <div class="similar-header">
            <span class="similar-heading">Similar titles available</span>
            <span class="similar-sub">in your library</span>
          </div>
          <div class="similar-grid">
            <div
              v-for="sim in simCards"
              :key="sim.id"
              class="sim-card"
              role="button"
              tabindex="0"
              :aria-label="`Open ${sim.title}`"
              @click="router.push(`/title/${sim.id}`)"
              @keydown.enter="router.push(`/title/${sim.id}`)"
              @keydown.space.prevent="router.push(`/title/${sim.id}`)"
            >
              <div
                class="sim-poster"
                :style="{ background: sim.ph.background }"
              >
                <div
                  class="sim-poster-motif"
                  :style="{ background: sim.ph.motif }"
                />
                <div
                  class="sim-poster-mono"
                  :style="{ color: sim.ph.glyphColor }"
                >{{ sim.mono }}</div>
                <img
                  v-show="!simFailed[sim.id]"
                  :src="`/api/titles/${sim.id}/poster`"
                  loading="lazy"
                  alt=""
                  class="sim-poster-img"
                  @error="simFailed[sim.id] = true"
                />
              </div>
              <div class="sim-title">{{ sim.title }}</div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
  <div v-else-if="loading" class="detail-state">Loading…</div>
  <div v-else-if="loadError === 'transient'" class="detail-state" data-test="detail-error">
    <p>Couldn't load this title.</p>
    <button class="back-btn" @click="retry">Retry</button>
    <button class="back-btn" @click="back">← Library</button>
  </div>
  <div v-else class="detail-state" data-test="detail-notfound">
    <p>Title not found.</p>
    <button class="back-btn" @click="back">← Library</button>
  </div>
</template>

<style scoped>
/* ---- Backdrop ---- */
.backdrop {
  position: relative;
  height: 360px;
  overflow: hidden;
}

.backdrop-img,
.poster-img,
.sim-poster-img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.backdrop-motif {
  position: absolute;
  top: -30%;
  right: -10%;
  width: 55%;
  height: 120%;
  border-radius: 50%;
  filter: blur(8px);
}

.backdrop-mono {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding-right: 8%;
  font-family: var(--font-ui, 'Hanken Grotesk', sans-serif);
  font-weight: 800;
  font-size: 200px;
  letter-spacing: -0.05em;
}

.scrim-bottom {
  position: absolute;
  inset: 0;
  background: linear-gradient(180deg, rgba(11, 12, 15, 0.2) 0%, rgba(11, 12, 15, 0.55) 55%, var(--bg-app, #0b0c0f) 100%);
}

.scrim-left {
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, var(--bg-app, #0b0c0f) 8%, transparent 55%);
}

.back-btn {
  position: absolute;
  top: 18px;
  left: 22px;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 8px 14px;
  background: rgba(8, 9, 11, 0.6);
  backdrop-filter: blur(8px);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 8px;
  color: var(--text-primary, #e9ebf0);
  font-family: var(--font-ui, 'Hanken Grotesk', sans-serif);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  z-index: 5;
  transition: border-color 0.16s ease;
}

.back-btn:hover {
  border-color: rgba(255, 255, 255, 0.3);
}

/* ---- Body ---- */
.body {
  max-width: 1080px;
  margin: -180px auto 0;
  padding: 0 40px 60px;
  position: relative;
  display: flex;
  gap: 34px;
  align-items: flex-start;
}

/* ---- Poster column ---- */
.poster-col {
  flex: none;
  width: 232px;
}

.poster {
  position: relative;
  aspect-ratio: 2 / 3;
  border-radius: 12px;
  overflow: hidden;
  border: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 24px 50px rgba(0, 0, 0, 0.55);
}

.poster-motif {
  position: absolute;
  top: -18%;
  right: -22%;
  width: 80%;
  height: 56%;
  border-radius: 50%;
  filter: blur(3px);
}

.poster-mono {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-ui, 'Hanken Grotesk', sans-serif);
  font-weight: 800;
  font-size: 72px;
  letter-spacing: -0.04em;
}

/* ---- Mark as watched button ---- */
.watched-btn {
  width: 100%;
  margin-top: 14px;
  height: 42px;
  background: var(--watched-bg, #f5c518);
  border: 1px solid var(--watched-border, #f5c518);
  border-radius: 9px;
  color: var(--watched-color, #1a1400);
  font-family: var(--font-ui, 'Hanken Grotesk', sans-serif);
  font-size: 13.5px;
  font-weight: 600;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  transition: opacity 0.16s ease;
}

.watched-btn:hover {
  opacity: 0.9;
}

.watched-btn--active {
  --watched-bg: rgba(245, 197, 24, 0.14);
  --watched-border: rgba(245, 197, 24, 0.4);
  --watched-color: #f5d24e;
}

/* ---- Rating well ---- */
.rating-well {
  margin-top: 14px;
  padding: 14px;
  background: var(--surface-1, #15171c);
  border: 1px solid rgba(255, 255, 255, 0.07);
  border-radius: 10px;
}

.rating-eyebrow {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--text-faint, #5f6570);
  margin-bottom: 9px;
}

.no-imdb-hint {
  margin: 9px 0 0;
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10.5px;
  color: var(--text-faint, #5f6570);
}

.userdata-error {
  margin: 10px 0 0;
  font-size: 12px;
  line-height: 1.35;
  color: var(--text-danger, #f5a3a3);
}

.watched-btn:disabled {
  cursor: default;
  opacity: 0.5;
}

/* ---- Info column ---- */
.info-col {
  flex: 1;
  min-width: 0;
  padding-top: 96px;
}

/* Badge row */
.badge-row {
  display: flex;
  align-items: center;
  gap: 9px;
  margin-bottom: 10px;
}

.svc-pill-wrap {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 999px;
}

.rating-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 10px;
  background: rgba(245, 197, 24, 0.1);
  border: 1px solid rgba(245, 197, 24, 0.25);
  border-radius: 999px;
}

.pill-mark {
  color: var(--accent, #f5c518);
  font-size: 11px;
}

.pill-score {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 11px;
  font-weight: 600;
  color: var(--accent-text, #f5d24e);
}

.pill-label {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10px;
  color: var(--accent-dim, #8a7a30);
}

.anilist-pill { background: rgba(2, 169, 255, 0.1); border-color: rgba(2, 169, 255, 0.25); }
.anilist-pill .pill-mark { color: #02a9ff; font-weight: 700; font-size: 10px; }
.anilist-pill .pill-label { color: #2b7fb0; }

/* Title + fact line */
.title-h1 {
  margin: 0;
  font-size: 38px;
  font-weight: 800;
  letter-spacing: -0.03em;
  line-height: 1.05;
  color: var(--text-strong, #f4f5f7);
}

.fact-line {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 12.5px;
  color: var(--text-muted, #8a909b);
  margin-top: 10px;
}

/* Description */
.desc {
  font-size: 15.5px;
  line-height: 1.65;
  color: var(--text-secondary, #c2c7d0);
  max-width: 600px;
  margin: 18px 0 0;
}

/* Cast */
.cast-section {
  margin-top: 22px;
}

.section-eyebrow {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--text-faint, #5f6570);
  margin-bottom: 9px;
}

.cast-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.cast-chip {
  padding: 6px 12px;
  background: var(--surface-1, #15171c);
  border: 1px solid rgba(255, 255, 255, 0.07);
  border-radius: 8px;
  font-size: 13px;
  color: var(--text-secondary, #c2c7d0);
}

/* Similar titles mini-grid */
.similar-section {
  margin-top: 32px;
}

.similar-header {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-bottom: 14px;
}

.similar-heading {
  font-size: 16px;
  font-weight: 700;
  color: var(--text-primary, #e9ebf0);
}

.similar-sub {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 11px;
  color: var(--text-faint, #5f6570);
}

.similar-grid {
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: 14px;
}

.sim-card {
  cursor: pointer;
  transition: transform 0.16s ease;
}

.sim-card:hover {
  transform: translateY(-3px);
}

.sim-card:focus-visible {
  outline: 2px solid var(--accent-line-2, rgba(245, 197, 24, 0.4));
  outline-offset: 3px;
  border-radius: var(--r-md, 8px);
}

.sim-poster {
  position: relative;
  aspect-ratio: 2 / 3;
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.sim-poster-motif {
  position: absolute;
  top: -20%;
  right: -25%;
  width: 80%;
  height: 60%;
  border-radius: 50%;
  filter: blur(2px);
}

.sim-poster-mono {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-ui, 'Hanken Grotesk', sans-serif);
  font-weight: 800;
  font-size: 34px;
}

.sim-title {
  margin-top: 7px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary-strong, #d2d6dd);
  line-height: 1.25;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.detail-state {
  padding: 80px 22px;
  text-align: center;
  color: var(--text-faint, #5f6570);
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
}
</style>
