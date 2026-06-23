# cue — Catalogue Browse Scaling (slim list + lazy detail + grid virtualization)

**Date:** 2026-06-23
**Status:** Design — awaiting review
**Source:** `docs/superpowers/deferred-followups.md` → Scale §, item 1
("`GET /api/catalogue` returns the entire library in one response").

## Goal

Make catalogue browse comfortably scale to cue's **bounded** catalogue ceiling
(design D1: Full Plex library ∪ entire UK Disney+ & Crunchyroll catalogues —
realistically low tens of thousands of titles) **without** disturbing the
deliberate load-everything-once interaction model (design spec §, line 109:
"Filters/sort/search compose on top of the answer set entirely in the Pinia
store").

Two independent levers, both in scope:

1. **Data weight** — trim the startup payload + reactive hydration cost.
2. **DOM weight** — stop rendering thousands of off-screen poster cards.

## Why this shape (and not server-side pagination)

Server-side pagination is the right answer for an *unbounded* catalogue. cue's
catalogue is **bounded by design** (D1), it is **single-user on localhost**, and
client-side filter/sort/search composition is an explicit D-level decision
because it makes the Ask → refine → filter → similar loop feel instant with zero
round-trips. Pagination would add cursor/facet/cache-invalidation complexity and
a round-trip per filter toggle to solve a scaling problem D1 guarantees we never
hit — while *removing* the instant feel. Rejected.

Crucially, **production Ask is already server-side.** `services/index.ts` wires
`ApiAskService`; `ask`/`refine`/`similar` send only `{query}` / `{kind, ids}` /
`{anchorId}` and ignore the in-memory catalogue. So the only client-side
consumers of the two heaviest per-title fields — `desc` and `cast` — are
`DetailView`. They can leave the list payload entirely.

## Non-goals

- No DB schema change / migration. `description`, `title_cast` stay in the DB;
  the list query simply stops reading them.
- No change to filter/sort/search/genre-dropdown/similar behaviour or to the Ask
  endpoints. The store keeps composing client-side over the full in-memory set.
- No server-side pagination, cursors, or facet endpoints.
- No virtualization library dependency (the project keeps a deliberate 4-dep
  runtime footprint).

---

## Part 1 — Slim list DTO + lazy detail endpoint (backend)

### DTO split

The list payload drops the two DetailView-only fields. Everything browse needs
(services, genres, imdb, len, year, type, title, watched, rating, imdb_id) stays.

- **`TitleListItem`** (new, in `models.rs`) — current `TitleDto` **minus**
  `desc` and `cast`.
- **`TitleDto`** (unchanged) — the full record, returned by the detail endpoint.

`imdb_id` stays in the list item (9 bytes; keeping it minimises frontend type
churn). The heavy fields (`desc` = largest column; `cast` = the only extra join)
are the entire win — together ~60–70% of the per-title bytes.

### `db::catalogue` changes

- `fetch_catalogue(pool) -> Vec<TitleListItem>`: drop the `description` column
  from the `titles` SELECT and **remove the `title_cast` query entirely**. Keeps
  the `title_services`, `title_genres`, `user_ratings`, `watch_history` reads.
- `fetch_title(pool, id: i64) -> anyhow::Result<Option<TitleDto>>` (new): fetch
  one fully-hydrated title — its row, its services/genres/cast, and its
  watched/rating (looked up by `imdb_id`). Returns `None` when no row has that
  `id`.

### Route

Add to `routes::configure`, inside the `/api` scope:

```
.route("/titles/{id}", web::get().to(catalogue::get_title))
```

`GET /api/titles/{id}` → `catalogue::get_title`: `200` full `TitleDto`, `404`
when `fetch_title` returns `None`, `500` on query error. No path conflict with
the existing `/titles/{id}/rating|watched|poster|backdrop` (different depth).

### Backend tests

- `catalogue_endpoint_returns_seed` (existing): keep the 28-row + shape asserts;
  add that list items have **no** `desc`/`cast` keys.
- `title_detail_returns_full_record` (new): a seeded id returns `200` with
  `desc` (string) and `cast` (array) present.
- `title_detail_unknown_id_is_404` (new).

---

## Part 2 — Frontend types + client (data weight)

- `@/types`: split the title shape.
  - `TitleListItem` — the list/store shape (no `desc`, no `cast`).
  - `TitleDetail = TitleListItem & { desc: string; cast: string[] }`.
  - Keep a `Title` alias = `TitleListItem` to minimise churn across the store,
    getters, PosterGrid, askService signatures (which only ever touch list
    fields).
- `@/api/client.ts`:
  - `getCatalogue(): Promise<TitleListItem[]>` — drop `desc`/`cast` from the
    `isTitle` validator.
  - `getTitle(id): Promise<TitleDetail>` (new) — `fetch('/api/titles/'+id)`,
    throw on `!ok` (with a distinguishable message for `404`), validate the
    detail shape (list fields + `desc` string + `cast` array).

The store catalogue becomes `TitleListItem[]`. `store.similar` already uses only
`genres`/`imdb`/`title`/`id`, so it is unaffected.

## Part 3 — DetailView fetches its own record

`DetailView` currently reads `store.catalogue.find(id)` (line 14), which after
the slim-down lacks `desc`/`cast`.

- Replace with a fetched `detail = ref<TitleDetail | null>` populated by
  `getTitle(id)` in `onMounted` (and on `id` change, for similar-card
  navigation within DetailView).
- States: **loading** (fetch in flight), **loaded** (render), **not-found**
  (`404` → a small "title not found" panel with a back link).
- `desc` and `cast` render from `detail`. `factLine`, poster/backdrop, monogram
  use `detail` (all fields present in the full DTO).
- `similar` still comes from `store.similar(id)` over the slim in-memory
  catalogue (genre overlap) — so the catalogue must be present for the similar
  row. Keep the existing `if (store.catalogue.length === 0) store.load()` guard
  alongside the detail fetch; the two run independently (detail does not block on
  the catalogue, and vice-versa).
- `canRate`/watched/rating read from `detail.imdbId` and the store maps as today.

### Frontend tests (Parts 2–3)

- `client.getTitle`: parses a valid detail; throws on `404`; throws on malformed.
- `DetailView`: renders `desc`/`cast` from a mocked `getTitle`; shows the
  not-found panel on `404`; similar row still renders from a seeded store
  catalogue; rating/watched controls behave as before.
- Update `catalogue` store / `isTitle` tests for the slimmed shape.

---

## Part 4 — Hand-rolled window-scroll grid virtualization (DOM weight)

Uniform `PosterCard` height (within a layout width) makes windowing pure
arithmetic — no per-row measurement cache, no library. One composable +
localised `PosterGrid` changes.

### Single source of truth for column count

Today the grid is `repeat(auto-fill, minmax(158px, 1fr))` — **CSS** decides the
column count. Virtualization needs **JS** to know the column count to map item
index ↔ row. To avoid CSS/JS desync, **JS becomes the source of truth**:

- Constants (shared module, e.g. `@/design/grid.ts`): `MIN_COL = 158`,
  `COL_GAP = 18`, `ROW_GAP = 22` — must mirror the CSS, kept in one place.
- `cols = max(1, floor((containerWidth + COL_GAP) / (MIN_COL + COL_GAP)))` —
  this *is* the auto-fill formula, so it picks exactly what auto-fill would.
- Grid CSS switches to `grid-template-columns: repeat(var(--cols), minmax(0, 1fr))`
  with `--cols` set from JS. Same visual result, deterministic for the math.

### `useVirtualGrid` composable

Inputs (refs): `containerEl`, `itemCount`, `overscanRows` (default 3).
Internally tracks two measured reactive values, plus window scroll:

- `containerWidth` — from a `ResizeObserver` on `containerEl` → derives `cols`.
- `rowHeight` — measured from a rendered card ref (`cardHeight + ROW_GAP`);
  uniform within a layout width, re-measured on resize. (Card width flexes via
  `1fr`, so aspect-locked card **height** changes with width — hence measure,
  don't assume a constant.)
- window `scroll`/`resize` (passive listeners) → `gridTop` (grid offset in the
  document) and `viewportH = innerHeight`.

Outputs (computed):

```
offset    = max(0, scrollY - gridTop)
totalRows = ceil(itemCount / cols)
startRow  = max(0, floor(offset / rowHeight) - overscanRows)
endRow    = min(totalRows, ceil((offset + viewportH) / rowHeight) + overscanRows)
startIndex = startRow * cols
endIndex   = min(itemCount, endRow * cols)
topSpacer    = startRow * rowHeight                 // px
bottomSpacer = (totalRows - endRow) * rowHeight     // px
```

Before `rowHeight` is measured, render the first batch (assume a small row
count) so a card exists to measure; correct on the next tick. To survive a
remount (back-nav from DetailView) without a height collapse that breaks native
scroll restoration, cache the last `rowHeight`/`cols` in module scope and seed
spacers from it immediately.

**Testability:** keep the `ResizeObserver`/window wiring in a thin setup layer
and expose the measured values as plain refs the unit tests set directly (jsdom
has no layout and no `ResizeObserver`). The window math is then tested as pure
functions of `(containerWidth, rowHeight, scrollY, gridTop, viewportH,
itemCount)`.

### `PosterGrid` rendering

Layout = a top spacer div, the grid of the visible slice, a bottom spacer div
(flex column). The grid keeps `.poster-grid` styling but with `--cols` driving
columns:

```
<div ref="containerEl" class="poster-grid-virtual">
  <div :style="{ height: topSpacer + 'px' }" />
  <div class="poster-grid" :style="{ '--cols': cols }">
    <PosterCard v-for="t in visibleSlice" :key="t.id" ... />
  </div>
  <div :style="{ height: bottomSpacer + 'px' }" />
</div>
```

`visibleSlice = titles.slice(startIndex, endIndex)`. Always-virtualize (single
code path); at small counts the spacers are `0` and everything renders.

### Virtualization tests

- Composable: window math correctness across positions; clamping at top/bottom;
  overscan; `cols`/`rowHeight` recompute; `itemCount` smaller than one viewport
  (spacers `0`, full render).
- `PosterGrid`: with injected measurements, renders only the windowed cards +
  both spacers, sets `--cols`. Provide a minimal `ResizeObserver` mock in the
  vitest setup.

---

## Risks & mitigations

- **CSS/JS column desync** → eliminated by making JS authoritative via `--cols`
  with the auto-fill formula and shared constants.
- **Card height varies with width** (aspect-locked, `1fr`) → measure `rowHeight`
  from a live card, re-measure on resize, rather than assuming a constant.
- **Scroll restoration on back-nav** → module-scope cache of `rowHeight`/`cols`
  seeds spacer height immediately on remount so the browser can restore scroll;
  if imperfect, a follow-up can persist exact scroll offset in the store.
- **Keyboard nav onto an off-screen card** → current keyboard handling (`/`
  focus ask, `Escape` clear) is document-level and unaffected; verify the
  `views/__tests__/keyboard.test.ts` suite still passes. If any card-level arrow
  nav exists, it is out of scope and noted.
- **jsdom has no layout/ResizeObserver** → composable designed for injectable
  measurements; tests drive refs directly + a small ResizeObserver mock.

## Verification

- Backend: `cargo clippy --all-targets -- -D warnings` + `cargo test` green;
  new detail-endpoint + slim-list assertions pass.
- Frontend: `npm run build` (vue-tsc typecheck) + `npm test` green.
- Manual (deferred live-verify): on a real ~5k catalogue, confirm the list
  payload is materially smaller (no `desc`/`cast`), DetailView still shows
  description + cast, and the grid renders a constant ~window of cards while
  scrolling the full library smoothly.

## Out of scope / future

- `GET /api/facets` (genre/service lists from a cheap aggregate) — only needed
  if deriving facets from the in-memory set ever becomes a cost; not now.
- Persisted exact scroll-offset restoration, if module-cache seeding proves
  insufficient.
