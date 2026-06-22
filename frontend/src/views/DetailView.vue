<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useCatalogueStore } from '@/stores/catalogue'
import ServicePill from '@/components/ServicePill.vue'
import StarRating from '@/components/StarRating.vue'
import { posterPlaceholder, monogram } from '@/design/tokens'

const route = useRoute()
const router = useRouter()
const store = useCatalogueStore()

const id = computed(() => Number(route.params.id))
const title = computed(() => store.catalogue.find(t => t.id === id.value) ?? null)

const ph = computed(() => title.value ? posterPlaceholder(title.value.title) : null)
const mono = computed(() => title.value ? monogram(title.value.title) : '')

const factLine = computed(() => {
  if (!title.value) return ''
  const kind = title.value.type === 'movie' ? 'Movie' : 'Series'
  return `${title.value.year} · ${kind} · ${title.value.len} · ${title.value.genres.join(', ')}`
})

const similar = computed(() => title.value ? store.similar(id.value) : [])

function back() { router.push('/') }

// Wrappers that resolve id.value in script scope so the template never
// passes the ComputedRef object itself into store methods.
const idIsWatched = computed(() => store.isWatched(id.value))
function toggleWatched() { store.toggleWatched(id.value) }
const idRating = computed(() => store.ratingOf(id.value))
function setRating(n: number) { store.setRating(id.value, n) }

onMounted(() => {
  if (store.catalogue.length === 0) {
    store.load()
  }
})
</script>

<template>
  <div v-if="title && ph" class="detail-root">
    <!-- Backdrop band -->
    <div class="backdrop" :style="{ background: ph.backdrop }">
      <!-- Motif blob -->
      <div class="backdrop-motif" :style="{ background: ph.motif }" />
      <!-- Large monogram -->
      <div class="backdrop-mono" :style="{ color: ph.glyphColor }">{{ mono }}</div>
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
        </div>
        <!-- Writes below are local only — persistence is Plan 5 -->

        <!-- Mark as watched button -->
        <button
          data-test="mark-watched"
          class="watched-btn"
          :class="{ 'watched-btn--active': idIsWatched }"
          @click="toggleWatched"
        >
          {{ idIsWatched ? '✓ Watched' : 'Mark as watched' }}
        </button>

        <!-- Your rating well -->
        <div class="rating-well">
          <div class="rating-eyebrow">Your rating</div>
          <StarRating
            :value="idRating"
            @set="setRating"
          />
        </div>
      </div>

      <!-- Right: info column -->
      <div class="info-col">
        <!-- Badge row: service pills + IMDb pill -->
        <div class="badge-row">
          <ServicePill
            v-for="svc in title.services"
            :key="svc"
            :service="svc"
            class="svc-pill-wrap"
          />
          <span v-if="title.imdb !== null" class="imdb-pill">
            <span class="imdb-star">★</span>
            <span class="imdb-score">{{ title.imdb }}</span>
            <span class="imdb-label">IMDb</span>
          </span>
        </div>

        <!-- Title -->
        <h1 class="title-h1">{{ title.title }}</h1>

        <!-- Fact line -->
        <div class="fact-line">{{ factLine }}</div>

        <!-- Description -->
        <p class="desc">{{ title.desc }}</p>

        <!-- Cast -->
        <div class="cast-section">
          <div class="section-eyebrow">Cast</div>
          <div class="cast-chips">
            <span v-for="person in title.cast" :key="person" class="cast-chip">{{ person }}</span>
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
              v-for="sim in similar"
              :key="sim.id"
              class="sim-card"
              @click="router.push(`/title/${sim.id}`)"
            >
              <div
                class="sim-poster"
                :style="{ background: posterPlaceholder(sim.title).background }"
              >
                <div
                  class="sim-poster-motif"
                  :style="{ background: posterPlaceholder(sim.title).motif }"
                />
                <div
                  class="sim-poster-mono"
                  :style="{ color: posterPlaceholder(sim.title).glyphColor }"
                >{{ monogram(sim.title) }}</div>
              </div>
              <div class="sim-title">{{ sim.title }}</div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ---- Backdrop ---- */
.backdrop {
  position: relative;
  height: 360px;
  overflow: hidden;
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

.imdb-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 10px;
  background: rgba(245, 197, 24, 0.1);
  border: 1px solid rgba(245, 197, 24, 0.25);
  border-radius: 999px;
}

.imdb-star {
  color: var(--accent, #f5c518);
  font-size: 11px;
}

.imdb-score {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 11px;
  font-weight: 600;
  color: var(--accent-text, #f5d24e);
}

.imdb-label {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10px;
  color: #8a7a30;
}

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
  color: #d2d6dd;
  line-height: 1.25;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
