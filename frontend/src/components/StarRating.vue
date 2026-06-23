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
  <div class="star-row" :class="{ 'star-row--disabled': props.disabled }">
    <button
      v-for="n in STARS"
      :key="n"
      data-test="star"
      :disabled="props.disabled"
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

.star-row--disabled .star {
  cursor: default;
  opacity: 0.5;
}
</style>
