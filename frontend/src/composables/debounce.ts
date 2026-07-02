/**
 * Returns a debounced wrapper of `fn`: calls are coalesced so `fn` runs once,
 * `delay` ms after the last call, with that call's arguments.
 */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  delay: number,
): (...args: A) => void {
  let timer: ReturnType<typeof setTimeout> | undefined
  return (...args: A) => {
    if (timer !== undefined) clearTimeout(timer)
    timer = setTimeout(() => {
      timer = undefined
      fn(...args)
    }, delay)
  }
}
