<script setup lang="ts">
import type { ServiceKey } from '@/types'
import { services } from '@/design/tokens'

const props = defineProps<{ id: number; watchable: ServiceKey[] }>()
</script>

<template>
  <div v-if="props.watchable.length" class="watch-links" data-test="watch-links">
    <div class="watch-eyebrow">Watch on</div>
    <a
      v-for="svc in props.watchable"
      :key="svc"
      class="watch-btn"
      :href="`/api/titles/${props.id}/watch/${svc}`"
      target="_blank"
      rel="noopener noreferrer"
      :data-test="`watch-${svc}`"
      :style="{ '--svc-dot': services[svc].dot }"
    >
      <span class="dot" />
      {{ services[svc].label }}
    </a>
  </div>
</template>

<style scoped>
.watch-links {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 12px;
}
.watch-eyebrow {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 10px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-secondary, #9aa0aa);
}
.watch-btn {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 8px 12px;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--svc-dot) 45%, transparent);
  background: color-mix(in srgb, var(--svc-dot) 12%, transparent);
  color: var(--text-primary, #f2f4f8);
  font-size: 12px;
  text-decoration: none;
  transition: background 120ms ease;
}
.watch-btn:hover {
  background: color-mix(in srgb, var(--svc-dot) 22%, transparent);
}
.watch-btn .dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--svc-dot);
  flex: none;
}
</style>
