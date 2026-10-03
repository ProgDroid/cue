import { MIN_COL, COL_GAP, ROW_GAP, META_BLOCK } from '@/design/grid'

export interface GridMetrics {
  containerWidth: number
  rowHeight: number   // card height + ROW_GAP; 0 until measured
  scrollOffset: number
  viewportH: number
  itemCount: number
  overscanRows: number
}

export interface GridWindow {
  cols: number
  startIndex: number
  endIndex: number
  topSpacer: number
  bottomSpacer: number
  totalRows: number
}

// Estimated row height used before the DOM has been measured. The poster is
// aspect-ratio 2/3, so posterHeight = cardWidth * 1.5; META_BLOCK covers the
// title + meta line below it. Rough is fine — overscan absorbs the error and
// measureRow() replaces this with the real height on the next tick.
export function estimateRowHeight(containerWidth: number, cols: number): number {
  if (containerWidth <= 0 || cols <= 0) return 0
  const cardWidth = (containerWidth + COL_GAP) / cols - COL_GAP
  return cardWidth * 1.5 + META_BLOCK + ROW_GAP
}

export function computeWindow(m: GridMetrics): GridWindow {
  const cols = m.containerWidth > 0
    ? Math.max(1, Math.floor((m.containerWidth + COL_GAP) / (MIN_COL + COL_GAP)))
    : 1
  const totalRows = Math.ceil(m.itemCount / cols)

  // Use the measured row height when available, otherwise an estimate. NEVER
  // fall back to rendering every item — that mounts the whole catalogue at once
  // (the first-paint freeze + OOM crash this fix exists to prevent).
  const rowHeight = m.rowHeight > 0 ? m.rowHeight : estimateRowHeight(m.containerWidth, cols)

  if (rowHeight <= 0) {
    // Width is unknown too (very first synchronous render, pre-measure). Render
    // a small first window so we never mount the entire list.
    const firstWindowEnd = Math.min(m.itemCount, cols * (m.overscanRows + 4))
    return { cols, startIndex: 0, endIndex: firstWindowEnd, topSpacer: 0, bottomSpacer: 0, totalRows }
  }

  const startRow = Math.max(0, Math.floor(m.scrollOffset / rowHeight) - m.overscanRows)
  const endRow = Math.min(totalRows, Math.ceil((m.scrollOffset + m.viewportH) / rowHeight) + m.overscanRows)
  const startIndex = startRow * cols
  const endIndex = Math.min(m.itemCount, endRow * cols)
  const topSpacer = startRow * rowHeight
  const bottomSpacer = Math.max(0, (totalRows - endRow) * rowHeight)

  return { cols, startIndex, endIndex, topSpacer, bottomSpacer, totalRows }
}
