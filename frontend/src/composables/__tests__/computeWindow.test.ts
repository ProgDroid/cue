import { describe, it, expect } from 'vitest'
import { computeWindow } from '@/composables/computeWindow'

const base = { containerWidth: 800, rowHeight: 172, scrollOffset: 0, viewportH: 400, itemCount: 100, overscanRows: 3 }

describe('computeWindow', () => {
  it('derives column count from width via the auto-fill formula', () => {
    // floor((800 + 18) / (158 + 18)) = floor(818/176) = 4
    expect(computeWindow(base).cols).toBe(4)
  })

  it('renders everything (spacers 0) before rowHeight is measured', () => {
    const w = computeWindow({ ...base, rowHeight: 0 })
    expect(w.startIndex).toBe(0)
    expect(w.endIndex).toBe(100)
    expect(w.topSpacer).toBe(0)
    expect(w.bottomSpacer).toBe(0)
  })

  it('windows to the visible rows + overscan at the top', () => {
    const w = computeWindow(base)
    // startRow = max(0, floor(0/172) - 3) = 0
    // endRow = min(25, ceil(400/172) + 3) = min(25, 3 + 3) = 6 ; cols 4 -> 24
    expect(w.startIndex).toBe(0)
    expect(w.endIndex).toBe(24)
    expect(w.topSpacer).toBe(0)
    expect(w.bottomSpacer).toBe((25 - 6) * 172)
  })

  it('windows in the middle with overscan both sides', () => {
    const w = computeWindow({ ...base, scrollOffset: 172 * 10 })
    // startRow = floor(1720/172) - 3 = 10 - 3 = 7 -> startIndex 28
    // endRow = ceil((1720+400)/172) + 3 = ceil(12.3) + 3 = 13 + 3 = 16 -> endIndex 64
    expect(w.startIndex).toBe(28)
    expect(w.endIndex).toBe(64)
    expect(w.topSpacer).toBe(7 * 172)
  })

  it('clamps at the bottom (no negative bottom spacer)', () => {
    const w = computeWindow({ ...base, scrollOffset: 172 * 1000 })
    expect(w.endIndex).toBe(100)
    expect(w.bottomSpacer).toBe(0)
  })

  it('falls back to one column at zero width', () => {
    expect(computeWindow({ ...base, containerWidth: 0 }).cols).toBe(1)
  })
})
