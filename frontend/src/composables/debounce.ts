/** A debounced function plus `cancel()` to drop any pending call. */
export type Debounced<A extends unknown[]> = ((...args: A) => void) & { cancel: () => void }

/**
 * Returns a debounced wrapper of `fn`: calls are coalesced so `fn` runs once,
 * `delay` ms after the last call, with that call's arguments.
 */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  delay: number,
): Debounced<A> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const cancel = () => {
    if (timer !== undefined) clearTimeout(timer)
    timer = undefined
  }
  const debounced = (...args: A) => {
    cancel()
    timer = setTimeout(() => {
      timer = undefined
      fn(...args)
    }, delay)
  }
  return Object.assign(debounced, { cancel })
}
