import { MIN_COL, COL_GAP } from '@/design/grid'

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

export function computeWindow(m: GridMetrics): GridWindow {
  const cols = m.containerWidth > 0
    ? Math.max(1, Math.floor((m.containerWidth + COL_GAP) / (MIN_COL + COL_GAP)))
    : 1
  const totalRows = Math.ceil(m.itemCount / cols)

  if (m.rowHeight <= 0) {
    return { cols, startIndex: 0, endIndex: m.itemCount, topSpacer: 0, bottomSpacer: 0, totalRows }
  }

  const startRow = Math.max(0, Math.floor(m.scrollOffset / m.rowHeight) - m.overscanRows)
  const endRow = Math.min(totalRows, Math.ceil((m.scrollOffset + m.viewportH) / m.rowHeight) + m.overscanRows)
  const startIndex = startRow * cols
  const endIndex = Math.min(m.itemCount, endRow * cols)
  const topSpacer = startRow * m.rowHeight
  const bottomSpacer = Math.max(0, (totalRows - endRow) * m.rowHeight)

  return { cols, startIndex, endIndex, topSpacer, bottomSpacer, totalRows }
}
