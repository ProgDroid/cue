<script setup lang="ts">
import { computed } from 'vue'
import type { Title } from '@/types'
import { posterPlaceholder, monogram, services } from '@/design/tokens'

const props = defineProps<{ title: Title; watched: boolean }>()
const emit = defineEmits<{ select: [id: number]; 'find-similar': [t: Title] }>()

const ph = computed(() => posterPlaceholder(props.title.title))
const mono = computed(() => monogram(props.title.title))

const metaLine = computed(() => `${props.title.year} · ${props.title.type}`)
</script>

<template>
  <div
    data-test="card"
    class="card"
    @click="emit('select', title.id)"
  >
    <!-- Poster block -->
    <div class="poster" :style="{ background: ph.background }">
      <!-- Motif radial gradient -->
      <div class="poster-motif" :style="{ background: ph.motif }" />

      <!-- Monogram -->
      <div class="poster-mono" :style="{ color: ph.glyphColor }">{{ mono }}</div>

      <!-- Vignette overlay -->
      <div class="poster-vignette" />

      <!-- Service badge (top-left) -->
      <div class="poster-service-badge">
        <span
          v-for="svcKey in title.services"
          :key="svcKey"
          class="svc-dot"
          :style="{ background: services[svcKey].dot }"
        />
      </div>

      <!-- IMDB rating badge (top-right) -->
      <div v-if="title.imdb !== null" class="poster-imdb-badge">
        <span class="imdb-star">★</span>
        <span class="imdb-score">{{ title.imdb }}</span>
      </div>

      <!-- Watched badge (bottom-right) -->
      <div v-if="watched" data-test="watched-badge" class="watched-badge">✓</div>
    </div>

    <!-- Meta row -->
    <div class="meta-row">
      <div class="meta-text">
        <div class="meta-title">{{ title.title }}</div>
        <div class="meta-line">{{ metaLine }}</div>
      </div>
      <button
        data-test="find-similar"
        class="find-similar-btn"
        title="Find similar in your library"
        @click.stop="emit('find-similar', title)"
      >✦</button>
    </div>
  </div>
</template>

<style scoped>
.card {
  cursor: pointer;
  transition: transform var(--dur-hover, 0.16s) var(--ease, cubic-bezier(0.4, 0, 0.2, 1));
  animation: cueFade 0.3s ease;
}

.card:hover {
  transform: translateY(-4px);
}

/* Poster block */
.poster {
  position: relative;
  aspect-ratio: 2 / 3;
  border-radius: var(--r-md, 8px);
  overflow: hidden;
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.poster-motif {
  position: absolute;
  top: -20%;
  right: -25%;
  width: 80%;
  height: 60%;
  border-radius: 50%;
  filter: blur(2px);
}

.poster-mono {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-ui, 'Hanken Grotesk', sans-serif);
  font-weight: 800;
  font-size: 52px;
  letter-spacing: -0.04em;
}

.poster-vignette {
  position: absolute;
  inset: 0;
  background: linear-gradient(
    180deg,
    rgba(0, 0, 0, 0.28) 0%,
    transparent 28%,
    transparent 60%,
    rgba(0, 0, 0, 0.55) 100%
  );
}

.poster-service-badge {
  position: absolute;
  top: 9px;
  left: 9px;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  background: rgba(8, 9, 11, 0.62);
  backdrop-filter: blur(6px);
  border-radius: var(--r-pill, 999px);
}

.svc-dot {
  display: inline-block;
  width: 7px;
  height: 7px;
  border-radius: 50%;
}

.poster-imdb-badge {
  position: absolute;
  top: 9px;
  right: 9px;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 7px;
  background: rgba(8, 9, 11, 0.62);
  backdrop-filter: blur(6px);
  border-radius: var(--r-sm, 6px);
}

.imdb-star {
  color: var(--accent, #f5c518);
  font-size: 10px;
}

.imdb-score {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 11px;
  font-weight: 600;
  color: var(--accent-text, #f5d24e);
}

.watched-badge {
  position: absolute;
  bottom: 9px;
  right: 9px;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--accent, #f5c518);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--accent-on, #1a1400);
  font-size: 12px;
  font-weight: 800;
}

/* Meta row below poster */
.meta-row {
  margin-top: 9px;
  display: flex;
  align-items: flex-start;
  gap: 6px;
}

.meta-text {
  flex: 1;
  min-width: 0;
}

.meta-title {
  font-size: var(--t-base, 13.5px);
  font-weight: 600;
  line-height: 1.25;
  color: var(--text-primary, #e9ebf0);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.meta-line {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: var(--t-meta, 11px);
  color: var(--text-faint, #5f6570);
  margin-top: 3px;
}

.find-similar-btn {
  flex: none;
  width: 24px;
  height: 24px;
  border-radius: var(--r-sm, 6px);
  background: var(--surface-3, #1d2129);
  border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
  color: var(--text-muted, #8a909b);
  font-size: 12px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  transition: color var(--dur-hover, 0.16s) var(--ease, cubic-bezier(0.4, 0, 0.2, 1)),
    border-color var(--dur-hover, 0.16s) var(--ease, cubic-bezier(0.4, 0, 0.2, 1));
}

.find-similar-btn:hover {
  color: var(--accent, #f5c518);
  border-color: var(--accent-line-2, rgba(245, 197, 24, 0.4));
}
</style>
