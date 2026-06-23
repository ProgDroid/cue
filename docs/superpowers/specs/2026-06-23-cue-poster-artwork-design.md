# cue — Real poster + backdrop artwork — design

**Date:** 2026-06-23
**Status:** Approved (brainstorming) — ready for implementation plan
**Source backlog item:** `docs/superpowers/deferred-followups.md` → Features → "Real poster
artwork — now actionable" + master spec §11.

## Goal

Replace the generated oklch placeholder posters/backdrops with **real artwork** captured
during catalogue sync, served through the backend so the Plex token never reaches the
browser. The existing placeholder remains the universal fallback for art-less titles, so
there is zero regression.

Render real art at every site that currently draws a placeholder:

1. `PosterCard` — grid card (vertical poster).
2. `DetailView` — poster column (vertical poster).
3. `DetailView` — backdrop / hero (horizontal backdrop).
4. `DetailView` — similar-titles row thumbnails (vertical poster).

## Decisions (locked during brainstorming)

- **D-PA1 — Plex artwork via server-side proxy.** MOTN poster URLs are public CDN links
  (`cdn.movieofthenight.com`) usable directly in `<img src>`. Plex `thumb`/`art` are
  *relative paths on the local Plex server* requiring `X-Plex-Token`. Per the project
  security boundary (CLAUDE.md: the Plex key is never serialized to the client), the
  frontend always targets a backend endpoint; the backend **302-redirects** to the public
  CDN URL for MOTN titles, or **streams the bytes with the token injected** for Plex
  titles. Token stays server-side.
- **D-PA2 — Poster *and* backdrop in scope.** Both the vertical poster and the horizontal
  backdrop are captured and rendered.
- **D-PA3 — Storage = 4 nullable columns on `titles`** (not a `title_images` table). The
  matrix is small and fixed (2 kinds × 2 source-types). Columns keep the upsert path and
  the endpoint lookup trivial and require no join on read.
- **D-PA4 — Frontend fetches art by id, not via the DTO.** `TitleDto` / the `Title` type /
  `fetch_catalogue` are **unchanged**. Only the new endpoint reads the image columns. The
  `<img>` overlays the existing placeholder and hides itself on load error, so the
  placeholder shows through for any 404.
- **D-PA5 — Prefer the public (remote) ref.** When a title is merged from *both* Plex and
  MOTN, the public CDN URL wins over the Plex token-proxy ref (cheaper 302, no token, CDN
  caching).
- **D-PA6 — Plex proxy-stream branch is live-verify, not mock-tested.** The redirect and
  404 branches are unit-testable without a live Plex. The byte-streaming branch is covered
  by a unit test on URL construction plus a manual live-verify note, consistent with how
  MOTN live-verify was deferred.

## Storage — migration `0004`

Add four nullable columns to `titles`:

| Column          | Meaning                                            | Source        |
| --------------- | -------------------------------------------------- | ------------- |
| `poster_url`    | Public absolute poster URL (302 target)            | MOTN CDN      |
| `poster_plex`   | Relative Plex thumb path (token-proxy target)      | Plex          |
| `backdrop_url`  | Public absolute backdrop URL (302 target)          | MOTN CDN      |
| `backdrop_plex` | Relative Plex art path (token-proxy target)        | Plex          |

```sql
ALTER TABLE titles ADD COLUMN poster_url    TEXT;
ALTER TABLE titles ADD COLUMN poster_plex   TEXT;
ALTER TABLE titles ADD COLUMN backdrop_url  TEXT;
ALTER TABLE titles ADD COLUMN backdrop_plex TEXT;
```

(End the file with a trailing newline — cosmetic lesson from `0002`.)

A title merged from both sources can populate the `_url` pair *and* the `_plex` pair; the
endpoint prefers `_url` (D-PA5).

## Sync parsing → `FetchedTitle`

Add two optional, source-tagged image refs to `FetchedTitle`:

```rust
/// One artwork reference plus how to serve it.
pub struct ImageRef {
    pub value: String, // public absolute URL (remote) OR relative Plex path
    pub remote: bool,  // true => public CDN (302); false => Plex path (token proxy)
}
// on FetchedTitle:
pub poster: Option<ImageRef>,
pub backdrop: Option<ImageRef>,
```

### MOTN (`src/sync/motn.rs`)

- Add `image_set: Option<ImageSet>` to the `Show` deserialize struct (camelCase `imageSet`).
- `ImageSet` mirrors the MOTN shape: `verticalPoster` and a horizontal image
  (`horizontalPoster` / backdrop), each a map of size keys (`w240`/`w360`/`w480`/`w600`/
  `w720`, plus larger keys for the horizontal image). **Exact size keys are locked at
  implementation time against a captured live `/changes` (or search) response**, which
  also becomes a parser fixture (closes the backlog's "capture a real `/changes` fixture"
  note).
- In `show_to_fetched`: pick the vertical poster at ~`w480` → `poster { remote: true }`;
  pick a horizontal image (largest reasonable, e.g. ~`w1080`) → `backdrop { remote: true }`.
  Missing `imageSet` → both `None`.

### Plex (`src/sync/plex.rs`)

- Add `thumb: Option<String>` and `art: Option<String>` to the `Meta` deserialize struct.
- In `parse_section`: `thumb` → `poster { remote: false }`, `art` → `backdrop { remote: false }`.

### Merge (`src/sync/merge.rs`)

- Carry `poster` / `backdrop` through `MergedTitle`.
- Fill rule (D-PA5): a `remote: true` ref **replaces** a previously-stored `remote: false`
  ref; otherwise first-seen wins (mirrors the existing scalar-fill behaviour, with the
  remote-preference tweak). At `upsert_title`, flatten each `ImageRef` into the right column
  pair: `remote: true` → `*_url`; `remote: false` → `*_plex`.

### MOTN delta cache (`src/db/motn_cache.rs`)

- Add `poster_url: Option<String>` and `backdrop_url: Option<String>` to `CachedTitle`
  (MOTN only ever produces `remote: true` refs, so no Plex fields needed here).
- `From<&FetchedTitle>` writes them; the cache→`FetchedTitle` reconstruction rebuilds
  `poster`/`backdrop` as `remote: true`. This keeps art on `/changes`-reconstructed
  snapshots.

### Store (`src/sync/store.rs`)

- Extend the `INSERT`/`UPDATE` in `upsert_title` to bind the 4 image columns from the
  flattened `MergedTitle` refs.

## The proxy endpoint (`src/routes/`)

Two routes:

- `GET /api/titles/{id}/poster`
- `GET /api/titles/{id}/backdrop`

Logic (per route, reading the matching column pair):

1. `SELECT poster_url, poster_plex FROM titles WHERE id = ?` (resp. backdrop pair).
2. Row missing → `404`.
3. `*_url` present → **302 redirect** to it, with `Cache-Control: public, max-age=…`.
4. else `*_plex` present → fetch `{plex_base}{path}?X-Plex-Token={token}` server-side with
   the existing reqwest client; on success **stream the bytes back** preserving the upstream
   `Content-Type` and adding `Cache-Control`. Upstream failure → `502`/`404`.
5. else → `404` (frontend falls back to the placeholder).

Needs `Config` (Plex base URL + token) and the pool from app state — same access pattern as
the sync route. Register both routes in `src/routes/mod.rs`.

## Frontend

- **No change to the `Title` type or any store** — art is fetched by id.
- Reusable pattern: an `<img>` layered over the existing placeholder block:

  ```vue
  <img
    :src="`/api/titles/${title.id}/poster`"
    loading="lazy"
    class="poster-img"
    @error="imgFailed = true"
    v-show="!imgFailed"
  />
  ```

  - `loading="lazy"` keeps the full-catalogue grid (~5k titles, pagination still deferred)
    cheap — the browser fetches only posters near the viewport.
  - On `error` (404 or load failure) the `<img>` hides and the placeholder underneath shows
    through. The placeholder code is untouched.
- Apply at all four sites:
  - `PosterCard.vue` — poster over the `.poster` block.
  - `DetailView.vue` — poster over the `.poster` block; backdrop `<img>` over `.backdrop`;
    similar-row thumbs over `.sim-poster`.
- `object-fit: cover` so real art fills the 2:3 (poster) / hero (backdrop) frames.

## Testing

**Rust (`cargo test --lib`, clippy `-D warnings`):**

- `parse_page` / `parse_page_entries` (MOTN): extend a fixture with `imageSet`; assert the
  resulting `poster`/`backdrop` are `remote: true` with the expected sized URL; assert a
  show *without* `imageSet` yields `None`.
- `parse_section` (Plex): extend a fixture with `thumb`/`art`; assert `poster`/`backdrop`
  are `remote: false` with the relative path; assert items without them yield `None`.
- `merge`: a title seen from Plex (`remote: false`) then MOTN (`remote: true`) ends with the
  remote ref (D-PA5); remote-then-plex keeps remote.
- `CachedTitle` round-trip preserves `poster_url`/`backdrop_url`.
- Endpoint: redirect branch (returns 302 + Location = the stored URL) and 404 branch
  (no row / no art) are unit-tested against a temp DB. The Plex proxy-stream branch is
  covered by a URL-construction unit test + live-verify note (D-PA6).
- Migration `0004` applies cleanly on a fresh temp DB (covered transitively by `init_pool`
  in every store test).

**Frontend (`vitest`):**

- `PosterCard`: renders `<img>` with `src="/api/titles/<id>/poster"` and `loading="lazy"`;
  firing `@error` hides the img (placeholder visible). Extend the existing `PosterCard.test.ts`.
- `DetailView`: poster + backdrop `<img>` present with correct srcs.

## Out of scope / deferred

- **Image caching to disk** (offline self-containment) — rejected in favour of the proxy.
- **`title_images` table** — rejected in favour of columns (D-PA3); revisit only if more
  image kinds/sizes are needed.
- **Pagination of `/api/catalogue`** — still deferred; `loading="lazy"` mitigates the
  ~5k-image grid in the meantime.
- **Mock-HTTP test of the Plex proxy-stream branch** — deferred to live-verify (D-PA6).

## Live-verify (post-merge, when convenient)

- Run one real sync; confirm a MOTN title's `/api/titles/:id/poster` 302-redirects to a
  `cdn.movieofthenight.com` URL that loads.
- Confirm a Plex-only title's poster streams through the backend (correct image, no token in
  the response URL the browser sees).
- Spot-check the grid + a detail page render real art with placeholders only where art is
  genuinely absent.
