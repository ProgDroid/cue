<script setup lang="ts">
import { computed, ref } from 'vue'

const props = defineProps<{ value: number | null; disabled?: boolean }>()
const emit = defineEmits<{ set: [n: number]; clear: [] }>()

const STARS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

// While hovering, preview the rating the click would set (it replaces the saved value).
const hovered = ref<number | null>(null)
const shown = computed(() => hovered.value ?? props.value)

function hover(n: number) {
  if (!props.disabled) hovered.value = n
}

function click(n: number) {
  if (props.disabled) return
  if (n === props.value) emit('clear')
  else emit('set', n)
}
</script>

<template>
  <div
    class="star-row"
    role="group"
    aria-label="Your rating"
    :class="{ 'star-row--disabled': props.disabled }"
    @mouseleave="hovered = null"
  >
    <button
      v-for="n in STARS"
      :key="n"
      data-test="star"
      :disabled="props.disabled"
      :aria-label="`Rate ${n} out of 10`"
      :aria-pressed="props.value === n"
      :class="['star', { filled: shown !== null && n <= shown }]"
      @mouseenter="hover(n)"
      @click="click(n)"
    >★</button>
  </div>
</template>

<style scoped>
.star-row {
  display: flex;
  gap: 3px;
}

.star {
  background: none;
  border: none;
  padding: 0;
  cursor: pointer;
  font-size: 20px;
  line-height: 1;
  color: var(--star-empty, #3a3f4a);
  transition: color 0.1s ease;
}

.star.filled {
  color: var(--star-filled, #f5c518);
}

.star:focus-visible {
  outline: 2px solid var(--accent-line-2, rgba(245, 197, 24, 0.4));
  outline-offset: 2px;
  border-radius: var(--r-sm, 6px);
}

.star-row--disabled .star {
  cursor: default;
  opacity: 0.5;
}
</style>
