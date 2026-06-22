<script setup lang="ts">
import type { ServiceKey } from '@/types'
import { services } from '@/design/tokens'

const props = defineProps<{ service: ServiceKey }>()
const svc = services[props.service]
</script>

<template>
  <span class="service-pill">
    <span class="dot" :style="{ background: svc.dot }" />
    <span class="label">{{ svc.label }}</span>
  </span>
</template>

<style scoped>
.service-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}

.dot {
  display: inline-block;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex: none;
}

.label {
  font-family: var(--font-mono, 'JetBrains Mono', monospace);
  font-size: 9.5px;
  letter-spacing: 0.03em;
  color: #d2d6dd;
}
</style>
