<script setup lang="ts">
const props = defineProps<{ value: number | null; disabled?: boolean }>()
const emit = defineEmits<{ set: [n: number]; clear: [] }>()

const STARS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

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
  >
    <button
      v-for="n in STARS"
      :key="n"
      data-test="star"
      :disabled="props.disabled"
      :aria-label="`Rate ${n} out of 10`"
      :aria-pressed="props.value === n"
      :class="['star', { filled: props.value !== null && n <= props.value }]"
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

.star:not(:disabled):hover {
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
