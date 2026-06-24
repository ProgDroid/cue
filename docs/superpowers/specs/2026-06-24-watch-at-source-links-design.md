# Watch-at-source deep links — design

**Date:** 2026-06-24
**Status:** Approved (brainstorm)
**Depends on:** catalogue sync (Plex + MOTN sources), poster artwork (image-proxy 302 pattern)

## Problem

cue shows *which* services carry a title (passive `ServicePill` badges in
`DetailView`) but offers no way to act on that — the user still has to leave
cue, open Plex / Crunchyroll / Disney+ themselves, and search for the title
again. We want a one-click "watch this here" affordance that opens the title's
page on the source service in a new tab.

## Goal

From a title's detail page, render a "Watch on …" button per available source.
Clicking it opens that title's page on the source (the user's self-hosted Plex
web UI, or the Crunchyroll/Disney+ web page) in a new tab.

## Non-goals (YAGNI)

- No watch links on the catalogue grid cards — detail view only.
- No "play"/resume deep link (MOTN `videoLink`) — link to the title *page* only.
- No per-episode Plex links — series link to the show page.
- No `app.plex.tv` universal-link flavor — the self-hosted server's own web UI
  is the target (chosen because `app.plex.tv` routing against a self-hosted
  instance is unverified).

## UX decision

The `DetailView` layout already separates **actions** (left poster column:
"Mark as watched", "Your rating") from **passive metadata** (right info column:
the `ServicePill` + rating-pill badge row). Watch links are an *action*, so they
live in the **left action column, directly under "Mark as watched"** — not as
clickable metadata pills (which would create the inconsistency of a clickable
"Plex" pill sitting beside a non-clickable "★ Rating" pill of identical
appearance).

The buttons reuse each service's brand color from `ServicePill`'s palette so
they stay visually tied to the passive badges across the gutter, while reading
unambiguously as actions.

## Architecture

### 1. Unified redirect endpoint

```
GET /api/titles/{id}/watch/{service}   →  302 Location: <real source URL>
```

- `{service}` ∈ `plex | crunchyroll | disney`.
- The handler resolves the title's per-service link **server-side** and 302s to
  it. The real destination URLs (internal Plex address + `machineId`, MOTN link
  strings) never enter the API JSON or the client bundle — consistent with
  cue's security boundary (external infra is server-side only, never serialized
  to the client). This mirrors the existing image-proxy 302 pattern
  (`GET /api/titles/{id}/poster`).
- Responses:
  - `302` with `Location` — link resolved.
  - `404` — title not found, **or** no link on file for that service.
  - `400` — `{service}` is not one of the three known values.

**Honest caveat:** a 302 keeps the destination out of the *payload and bundle*,
but once the browser follows it the new tab's address bar shows the real URL.
That is unavoidable. What the redirect buys: nothing sensitive in the SPA, no
internal server address in JSON, and one central place that owns availability +
fallback.

### 2. Link construction (backend)

- **Plex:**
  ```
  {PLEX_WEB_URL}/web/index.html#!/server/{machineId}/details?key=%2Flibrary%2Fmetadata%2F{ratingKey}
  ```
  - `machineId` = the server's `machineIdentifier`, fetched once from Plex
    `/identity` during sync and cached in `app_meta`.
  - `ratingKey` = the per-title Plex item key, captured during sync.
  - The `key` query value is the URL-encoded `/library/metadata/{ratingKey}`.
- **Crunchyroll / Disney+:** 302 straight to the stored MOTN `link`.

### 3. `PLEX_WEB_URL` config

New **optional** config var, **defaults to `PLEX_URL`**. The backend reaches
Plex at `PLEX_URL` (which in a Docker setup may be a container-internal
hostname), but the *browser* needs a reachable address. For a plain self-hosted
box on a LAN the two are identical and the operator sets nothing; the override
exists only for the split-address (Docker) case. Server-side `Config` only;
never serialized to the client.

### 4. Data capture (sync layer)

- **MOTN** (`src/sync/motn.rs`):
  - Add `link: Option<String>` to `StreamOption`.
  - Carry a per-attributed-service link from `streamingOptions[country]` through
    `show_to_fetched` onto `FetchedTitle`. Links exist **only** when
    `streamingOptions[country]` is present; the existing fall-back-to-searched-
    services path (no availability data) yields no link, so that service is
    simply not "watchable".
- **Plex** (`src/sync/plex.rs`):
  - Deserialize the `ratingKey` field on `Meta`; carry it on `FetchedTitle`.
  - Fetch `machineIdentifier` once per sync run from `/identity`; persist to
    `app_meta`. Failure is **non-fatal** — log and keep the previously cached
    value; the rest of the sync proceeds.

`FetchedTitle` gains:
- `plex_rating_key: Option<String>`
- per-service links — modeled as `services: Vec<(Service, Option<String>)>` (or
  an equivalent parallel map); exact shape settled in the plan.

### 5. Storage (one new migration)

```sql
ALTER TABLE title_services ADD COLUMN link TEXT;            -- MOTN link; NULL for Plex
ALTER TABLE titles         ADD COLUMN plex_rating_key TEXT; -- NULL for non-Plex titles
CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);                                                          -- holds 'plex_machine_id'
```

`store.rs::reconcile_service` is extended to persist the per-`(title, service)`
link alongside the membership row. The titles upsert persists
`plex_rating_key`.

> Migration note (per CLAUDE.md): after adding the migration file, run
> `cargo clean -p cue` before `cargo test` so `sqlx::migrate!` re-embeds it.

### 6. Frontend

- The **detail** payload (`GET /api/titles/{id}`) gains a server-derived
  `watchable: string[]` — the list of services that actually resolved to a link
  (Plex listed iff `plex_rating_key` is set **and** `app_meta.plex_machine_id`
  is known; Crunchyroll/Disney listed iff their `title_services.link` is
  non-NULL). **No URLs** in the payload — availability only.
- New `WatchLinks.vue` block in the `DetailView` left action column, under
  "Mark as watched": one button per `watchable` service, brand-colored via the
  `ServicePill` palette, each rendered as
  `<a href="/api/titles/{id}/watch/{svc}" target="_blank" rel="noopener noreferrer">`.
- Renders nothing when `watchable` is empty.
- The **slim** catalogue/grid list payload is unchanged — links are detail-only.

## Data flow

```
sync (Plex)   ─ ratingKey per item ──┐
              ─ machineId (/identity) ┴─► app_meta.plex_machine_id
                                          titles.plex_rating_key

sync (MOTN)   ─ streamingOptions[].link ─► title_services.link (per service)

GET /api/titles/{id}      ─► detail payload incl. watchable: string[]   (no URLs)
                                   │
frontend WatchLinks.vue ──────────┘  renders <a> per watchable service
        │  click
        ▼
GET /api/titles/{id}/watch/{service}
        │  resolve link server-side
        ▼
302 → {PLEX_WEB_URL}/web/...#!/server/{machineId}/details?key=...   (Plex)
   → MOTN link                                                      (Crunchyroll/Disney)
```

## Error handling & fallbacks

- No link for a source → absent from `watchable` → no button (never a dead one).
- `machineId` unknown (never synced / Plex down at sync time) → Plex excluded
  from `watchable`.
- Redirect handler re-validates at click time and `404`s if the link vanished
  between page load and click.
- `machineId` fetch failure during sync is non-fatal (logged, prior value kept).

## Testing

- `motn.rs`: parse `link` from a fixture whose `streamingOptions[]` includes
  `link`; assert per-service attribution and that the no-availability fallback
  yields no link.
- `plex.rs`: parse `ratingKey`; unit-test the URL builder (machineId +
  ratingKey + `key` URL-encoding).
- Redirect route: 302 to the correct `Location` per service; 404 on missing
  link / unknown id; 400 on unknown service.
- `watchable` derivation: a service appears only when its link resolves.
- Frontend: `WatchLinks.vue` renders the right buttons/hrefs from `watchable`,
  and nothing when it is empty.

## Security

- No URLs in any client-facing payload; only the `watchable` availability list.
- `PLEX_WEB_URL`, `machineId`, and the internal `PLEX_URL` stay server-side.
- Outbound `<a target="_blank">` uses `rel="noopener noreferrer"`.
- MOTN `link` and the Plex web URL carry no auth token (Plex web auth is the
  user's own browser session) — unlike the image proxy, no token-stream needed.
