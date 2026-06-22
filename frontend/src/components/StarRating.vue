<script setup lang="ts">
const props = defineProps<{ value: number | null }>()
const emit = defineEmits<{ set: [n: number] }>()

const STARS = [1, 2, 3, 4, 5]
</script>

<template>
  <div class="star-row">
    <button
      v-for="n in STARS"
      :key="n"
      data-test="star"
      :class="['star', { filled: props.value !== null && n <= props.value }]"
      @click="emit('set', n)"
    >★</button>
  </div>
</template>

<style scoped>
.star-row {
  display: flex;
  gap: 5px;
}

.star {
  background: none;
  border: none;
  padding: 0;
  cursor: pointer;
  font-size: 24px;
  line-height: 1;
  color: var(--star-empty, #3a3f4a);
  transition: color 0.1s ease;
}

.star.filled {
  color: var(--star-filled, #f5c518);
}

.star:hover {
  color: var(--star-filled, #f5c518);
}
</style>
