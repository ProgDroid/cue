import { describe, it, expect, vi, afterEach } from 'vitest'
import { debounce } from '@/composables/debounce'

afterEach(() => vi.useRealTimers())

describe('debounce', () => {
  it('invokes only once after the delay with the latest args', () => {
    vi.useFakeTimers()
    const spy = vi.fn()
    const d = debounce(spy, 120)
    d('a'); d('b'); d('c')
    expect(spy).not.toHaveBeenCalled()
    vi.advanceTimersByTime(120)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith('c')
  })

  it('resets the timer on each call', () => {
    vi.useFakeTimers()
    const spy = vi.fn()
    const d = debounce(spy, 120)
    d('x')
    vi.advanceTimersByTime(100)
    d('y')
    vi.advanceTimersByTime(100)
    expect(spy).not.toHaveBeenCalled()
    vi.advanceTimersByTime(20)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith('y')
  })
})
