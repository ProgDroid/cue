import { ref, watch, onUnmounted, type Ref } from 'vue'

/**
 * Returns a boolean ref that becomes true only after `source` has been
 * continuously truthy for `delayMs` (anti-flicker), and resets to false the
 * moment `source` becomes falsy. A pending raise is cancelled if the source
 * clears first. Must be called from a component setup().
 */
export function useDelayedFlag(
  source: Ref<boolean> | (() => boolean),
  delayMs: number,
): Ref<boolean> {
  const flag = ref(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  const getter = typeof source === 'function' ? source : () => source.value

  const clear = () => {
    if (timer !== undefined) {
      clearTimeout(timer)
      timer = undefined
    }
  }

  watch(
    getter,
    (active) => {
      if (active) {
        if (timer === undefined) {
          timer = setTimeout(() => {
            timer = undefined
            flag.value = true
          }, delayMs)
        }
      } else {
        clear()
        flag.value = false
      }
    },
    { immediate: true },
  )

  onUnmounted(clear)
  return flag
}
