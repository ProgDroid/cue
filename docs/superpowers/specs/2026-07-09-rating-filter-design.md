# Rating filter for the catalogue — design

**Date:** 2026-07-09
**Status:** Approved, ready for implementation plan
**Scope:** Frontend-only. No backend endpoint, no migration.

## Problem

The catalogue Browse view can filter by service, type, genre, and free-text
search, and can *sort* by "Top rated" — but it cannot *filter* by rating. A user
who wants "only show me titles rated 7+" has no way to narrow the grid to
well-reviewed titles.

## Goal

Add an `X+` rating-threshold dropdown to the catalogue filter bar that hides
titles below the selected external rating.

## Decisions

- **Which rating:** a unified **best-available external rating** on a 0–10
  scale. Personal ratings are explicitly out of scope. AniList's 0–100 scale is
  normalised into the same 0–10 space so one threshold covers every title,
  including anime.
- **Unrated titles:** when a threshold is active, titles with *no* external
  rating (neither `score` nor `anilistScore`) are **hidden**. A threshold means
  "proven to be at least this good," not "not known to be worse."
- **Threshold values:** `Any` (default, no filtering), `6+`, `7+`, `8+`, `9+`.
  Whole-number steps focused on the useful top end.
- **Placement:** a fourth `<select>` in `FilterBar`, styled identically to the
  Genre and Sort selects, positioned immediately after the Sort select.

## Design

### 1. Rating metric — pure helper

A single function derives the unified external rating per title. It lives next to
the catalogue store so the filter (and any future consumer) share one definition.

```ts
// returns null when the title has no external rating at all
export function externalRating(t: Title): number | null {
  if (t.score != null) return t.score                    // 0–10, preferred
  if (t.anilistScore != null) return t.anilistScore / 10 // 0–100 → 0–10
  return null
}
```

Note the precedence differs intentionally from `PosterCard`'s badge, which
prefers AniList for *display*. The filter prefers `score` because it wants a
single comparable 0–10 scale; the badge is a display-preference choice. Both are
correct for their purpose.

### 2. Store — state, action, filter step

- New state field: `minRating: number`, default `0` (meaning "Any" / no
  filtering).
- New action: `setMinRating(n: number)`.
- One new step in the `visibleTitles` getter, added **after** the genre filter
  and before sorting:

```ts
if (state.minRating > 0)
  out = out.filter(t => {
    const r = externalRating(t)
    return r != null && r >= state.minRating
  })
```

Because the guard requires `r != null`, unrated titles drop out whenever a
threshold is active (the "hide them" decision). `minRating === 0` short-circuits,
so the default path costs nothing.

Slotting into the existing pipeline means the rating filter composes
automatically with the service / type / genre / search filters **and** with an
active Ask result — the rating threshold narrows Ask results too, consistent with
how every other filter behaves.

### 3. UI — one dropdown in `FilterBar`

A fourth `<select>`, styled identically to the existing Genre and Sort selects,
placed right after the Sort select:

```
[ All | Plex | Disney | Crunchyroll ]  [ All | Movies | Series ]   ⌄ All genres   ⌄ Top rated   ⌄ Any rating          142 titles
```

- Options: `Any rating` (value `0`), `6+` (`6`), `7+` (`7`), `8+` (`8`),
  `9+` (`9`).
- Two-way bound via a `computed` get/set to `store.minRating` /
  `store.setMinRating`, exactly like the existing `selectedGenre` /
  `selectedSort` bindings.
- `data-test="rating-select"` for parity with the other selects.

## Testing (TDD)

Store (`catalogue.test.ts`):
- A threshold hides titles with no external rating.
- AniList fallback compares on the normalised scale: a title with
  `anilistScore: 85` passes `8+` but fails `9+`.
- `minRating: 0` returns the full set (no filtering).
- The rating filter composes with a genre/service filter.
- The rating filter narrows an active Ask result.

Component (`FilterBar.test.ts`):
- Selecting `8+` calls `setMinRating(8)`.
- The `Any rating / 6+ / 7+ / 8+ / 9+` options render.

## Out of scope

- Personal-rating filtering.
- Backend `/api/catalogue` query params or server-side filtering (catalogue is
  bounded and composed client-side, per the standing pagination-deferred
  decision).
- Half-step thresholds.
