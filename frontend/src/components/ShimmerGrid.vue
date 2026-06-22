<script setup lang="ts">
// Shimmer grid shown while an ask/refine is resolving.
// Uses the cuePulse keyframe from the prototype (0%/100% opacity 0.45, 50% opacity 0.8).
const SHIMMER_COUNT = 10
</script>

<template>
  <div class="shimmer-grid">
    <div
      v-for="n in SHIMMER_COUNT"
      :key="n"
      class="shimmer-card"
    >
      <div class="shimmer-poster" />
      <div class="shimmer-title" />
      <div class="shimmer-meta" />
    </div>
  </div>
</template>

<style scoped>
@keyframes cuePulse {
  0%, 100% { opacity: 0.45; }
  50%       { opacity: 0.8; }
}

.shimmer-grid {
  padding: 6px 22px 40px;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: 22px 18px;
}

.shimmer-card {
  /* no extra styles needed — children carry the pulse */
}

.shimmer-poster {
  aspect-ratio: 2 / 3;
  border-radius: 8px;
  background: #15171c;
  animation: cuePulse 1.1s ease-in-out infinite;
}

.shimmer-title {
  height: 10px;
  width: 78%;
  border-radius: 4px;
  background: #15171c;
  margin-top: 11px;
  animation: cuePulse 1.1s ease-in-out infinite;
}

.shimmer-meta {
  height: 8px;
  width: 44%;
  border-radius: 4px;
  background: #15171c;
  margin-top: 7px;
  animation: cuePulse 1.1s ease-in-out infinite;
}
</style>
