// Mirror of the `.poster-grid` CSS. JS owns the column count for virtualization,
// so these MUST match the stylesheet values.
export const MIN_COL = 158 // px — minmax() floor
export const COL_GAP = 18  // px — column gap
export const ROW_GAP = 22  // px — row gap

// px — approximate height of the title + meta line rendered under each poster.
// Used only to estimate row height before the DOM is measured; the real height
// replaces it on the next tick, so a rough value is fine.
export const META_BLOCK = 56
