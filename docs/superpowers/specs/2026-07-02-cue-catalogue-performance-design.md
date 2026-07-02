# cue — Catalogue Page Performance & Loading UX

**Date:** 2026-07-02
**Status:** Design — approved, pending spec review
**Scope:** Frontend only (`frontend/src`). No backend, no migration, no API change.

## Problem

With a full library (~5,000 titles) the Browse page freezes on first paint, stutters
while scrolling, and eventually crashes the tab (out of memory). There is also no
loading indicator during the initial catalogue fetch — the page shows the "Nothing in
your library matches those filters" empty-state text while `status === 'loading'`,
which reads as a broken/empty library.

## Root causes (confirmed by reading the code)

1. **Virtualizer renders the entire catalogue before it is measured.**
   `computeWindow` (`frontend/src/composables/computeWindow.ts:27`) returns
   `endIndex: itemCount` whenever `rowHeight <= 0`. `rowHeight` is `0` on a cold first
   load until `measureRow()` runs a tick later (`cachedRowHeight` covers back-nav
   remounts but not the first paint of a session). So the first render mounts all ~5,000
   `PosterCard`s at once — 5,000 DOM subtrees, each with a `backdrop-filter: blur` badge
   and an `<img>` requesting `/api/titles/{id}/poster`. This is the first-paint freeze
   and the primary driver of the OOM crash.

2. **The catalogue is stored deeply reactive.**
   `catalogue.ts` assigns the fetched array to Pinia state (`catalogue: Title[]`), so Vue
   proxies every title object and every nested `genres`/`services` array — ~20k+ proxies
   created synchronously inside `load()`. The catalogue is immutable after load
   (watched/ratings already live in *separate* reactive maps, `catalogue.ts:21-22`), so
   this reactivity buys nothing and adds a main-thread stall plus retained memory.

3. **No initial-load indicator.** BrowseView only shows `ShimmerGrid` while
   `store.resolving` (Ask), not during `status === 'loading'` (initial fetch).

**Not a primary cause:** the per-keystroke `visibleTitles` re-filter/re-sort. Typing was
not a reported symptom. Addressed cheaply (debounce) but not the focus.

## Non-goals

- **Server-side pagination / filtering.** Deliberately deferred. The original reason to
  avoid pagination — "the LLM needs every title on the page" — is already moot:
  `ApiAskService.ask()` (`frontend/src/services/askService.ts:76`) sends only `{ query }`;
  the Ask engine runs server-side over the full DB and returns `ids`. The client holds the
  full catalogue only to *render* those ids and to apply local filters/sorts. The four
  changes below should resolve the crashes without overturning the D1 "bounded client
  catalogue" decision. Pagination stays parked as a documented fallback, reconsidered only
  if the post-fix measurement (below) shows it is still needed.

## Design

Four frontend changes, no architecture shift.

### 1. Estimated row height so the first frame is already windowed

Replace the `rowHeight <= 0 → render everything` branch in `computeWindow` with an
**estimated** row height so the window is always bounded.

- Add a pure helper `estimateRowHeight(containerWidth, cols)`:
  - `cardWidth = (containerWidth + COL_GAP) / cols - COL_GAP`
  - `posterHeight = cardWidth * 1.5` (poster is `aspect-ratio: 2 / 3`)
  - `rowHeight ≈ posterHeight + META_BLOCK + ROW_GAP`, where `META_BLOCK ≈ 56` (meta
    margin-top + up to 2 title lines + meta line). A rough estimate is fine — overscan
    absorbs the error and the real `measureRow()` corrects it on the next tick.
- `computeWindow` uses the measured/cached `rowHeight` when `> 0`, otherwise the estimate.
  It must **never** return `endIndex === itemCount` for a large set purely because it has
  not measured yet. When `containerWidth` is also unknown (cols would be 1), fall back to a
  small fixed first window (e.g. `endIndex = min(itemCount, cols * (viewport-rows + overscan))`
  with a conservative assumed row height) rather than the whole list.
- `useVirtualGrid` seeds `rowHeight` from `cachedRowHeight` when available (unchanged);
  the estimate covers the cold path where the cache is empty.

### 2. Non-reactive catalogue

In `load()`, freeze the titles before assigning so they are stored as plain objects:

- `Object.freeze` each title (and/or `markRaw` the array) prior to
  `this.catalogue = ...`. Getters (`visibleTitles`, `genres`, `similar`) read frozen
  objects without change. Watched/ratings continue to live in their reactive maps, so
  user-data toggles still update the UI.
- Result: no synchronous deep-proxy pass on ~5k objects, and a much smaller retained heap.

### 3. Initial-load indicator with anti-flicker threshold

- Reuse `ShimmerGrid` (matches grid columns → no layout shift when posters replace it).
- New composable `useDelayedFlag(source, delayMs)`: returns a ref that becomes `true` only
  after `source` has been continuously truthy for `delayMs`; resets immediately when
  `source` goes false, and cancels the pending timer if `source` clears before the delay.
- BrowseView shows `ShimmerGrid` when `useDelayedFlag(status === 'loading', 180)` is true.
  Fast loads (< 180 ms) never flash the shimmer. The empty-state text must only render when
  `status === 'ready'` (not during loading), fixing the misleading message.

### 4. Debounced text filter (~120 ms)

- Debounce the query used by `visibleTitles` so a 5k re-filter/re-sort does not run on every
  keystroke. The input stays controlled/responsive; only the derived filter value is
  debounced. Keep it small and self-contained (reuse `useDelayedFlag`'s timer pattern or a
  minimal debounce util).

## Components touched

| File | Change |
|---|---|
| `composables/computeWindow.ts` | Estimated-height fallback; never render-all when unmeasured |
| `composables/useVirtualGrid.ts` | Pass container width/cols to the estimate; seed logic unchanged |
| `composables/useDelayedFlag.ts` (new) | Threshold flag for load indicator + debounce timer |
| `stores/catalogue.ts` | Freeze/`markRaw` catalogue in `load()`; debounce query feeding `visibleTitles` |
| `views/BrowseView.vue` | Show `ShimmerGrid` on delayed `loading`; gate empty-state to `ready` |

## Testing

- **`computeWindow`**: with `rowHeight === 0` and a large `itemCount`, returns a bounded
  window (regression: assert `endIndex < itemCount`). Estimated-height helper tested for
  plausible values across widths.
- **`useDelayedFlag`**: flag false before threshold; true after; cancels/reset when source
  clears before the delay (fake timers).
- **`catalogue` store**: after `load()`, catalogue titles are frozen / not deeply reactive;
  watched/ratings maps still drive `isWatched`/`ratingOf`; debounced query still yields the
  correct `visibleTitles` after the debounce window.
- **BrowseView**: renders `ShimmerGrid` (not empty-state) while loading past threshold;
  renders empty-state only when `ready` and no matches.
- Existing PosterGrid/virtualization tests must stay green.

## Measurement checkpoint (per "lighten first, then measure")

After the four changes land, measure with the full ~5k set:
- Time-to-interactive on first paint (no long freeze).
- Peak tab memory during a full scroll-through (no OOM).
- Scroll smoothness.

Only if this is still inadequate do we revisit server-side pagination (which would then
also need a "hydrate titles by ids" endpoint so Ask results can render). Record the numbers
in `docs/superpowers/deferred-followups.md`.
