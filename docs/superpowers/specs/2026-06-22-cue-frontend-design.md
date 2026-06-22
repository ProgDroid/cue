# cue — Frontend Design Spec (Plan 2)

**Date:** 2026-06-22
**Status:** Approved (brainstorming) — pending implementation plan
**Predecessor:** `2026-06-21-cue-design.md` (overall architecture, decisions D1–D9)
**Design source:** `design_handoff_cue/` — `cue.dc.html` prototype + `tokens.css` / `tokens.ts` + `README.md`

---

## 1. Overview

Plan 2 builds the **frontend** for cue: a self-hosted, dark-themed, desktop-only,
mouse-navigated media-discovery SPA. It recreates the high-fidelity design in
`design_handoff_cue/` as a **Vue 3 + TypeScript + Pinia** app under `frontend/`,
served by the existing Rust/Actix backend as static files (the `serve_spa`
catch-all already falls back to `index.html`).

The design is **two screens plus one integration pattern**:
1. **Browse** (primary) — filterable poster grid with a persistent natural-language ask bar.
2. **Detail** — expanded title info, mark-as-watched, personal rating, "similar in your library."
3. The **Integrated ask** pattern — a sticky ask bar that reshapes the grid in place (never a modal/route).

**Fidelity is high.** Colors, typography, spacing, radii, and interactions are
final and recreated from the provided tokens. Ship **only the "Integrated"
concept**; drop the prototype's bottom chat-concept switcher and the
Unified/Side-panel layouts (they remain in the prototype for reference only).

### Stack
- Vue 3 (`<script setup>`) + TypeScript, built with **Vite**, package manager **npm**
  (matches the Dockerfile's `npm ci` / `frontend/dist`).
- **Pinia** for state, **Vue Router** for navigation.
- Bespoke styling from the handoff tokens — no Tailwind / component library.
- **Vitest + Vue Test Utils** for tests.

---

## 2. Key decisions (from brainstorming, 2026-06-22)

- **F1 — Live data.** The Browse grid fetches the real backend `GET /api/catalogue`
  (28 seeded titles today). The frontend is typed against the **real `TitleDto`**,
  not the prototype's simpler `Title`. Browse, filters, sort, search, and Detail
  are fully live.
- **F2 — Real contract, not the prototype's.** The backend DTO is richer than the
  prototype assumed: `services` is an **array** (`ServiceKey[]`), `imdb` is
  **nullable**, and `watched` / `rating` / `imdbId` are already included in the
  response. The frontend models all of these. Service keys (`plex` / `disney` /
  `crunchyroll`) already match the prototype's `tokens.ts`, so only the
  single→array shift matters.
- **F3 — Ask via a swappable seam.** The full integrated-ask UI is built now,
  wired to a temporary client-side **`StubAskService`** (the prototype's
  keyword/genre rules). Plan 3 swaps in an `ApiAskService` calling
  `POST /api/ask` at **one constructor seam** — store and components untouched.
- **F4 — Vue Router with real URLs.** `/` (Browse) and `/title/:id` (Detail).
  Real browser back/forward, deep-linkable detail pages. Filter/answer/thread
  state lives in the Pinia store and survives navigation.
- **F5 — One cohesive spec, phased implementation.** The screens share the store
  and tokens too heavily to split into separate plans; the implementation plan
  sequences three phases (Foundation+Browse → Detail → Ask).
- **F6 — Self-hosted fonts.** Hanken Grotesk + JetBrains Mono are bundled, not
  loaded from a CDN — consistent with a self-hosted, privacy-minded app.
- **F7 — Read-only user data.** `watched` / `rating` render from the catalogue
  response and can be toggled locally for feedback, but **persistence is Plan 5**
  (`POST` ratings/watched endpoints do not exist yet). No optimistic write that
  silently drops on reload is shipped as "done."

---

## 3. Project structure

```
frontend/
├── index.html
├── package.json
├── vite.config.ts          # dev proxy /api -> 127.0.0.1:8080
├── tsconfig.json
└── src/
    ├── main.ts             # app bootstrap (Pinia + Router)
    ├── App.vue             # shell: header + <RouterView>
    ├── design/
    │   ├── tokens.ts       # from handoff (verbatim)
    │   ├── tokens.css      # from handoff (global CSS custom properties)
    │   └── fonts.css       # @font-face for self-hosted fonts
    ├── assets/fonts/       # Hanken Grotesk + JetBrains Mono (woff2)
    ├── types.ts            # Title, ServiceKey, AskResult, ThreadStep…
    ├── api/
    │   └── client.ts       # getCatalogue()
    ├── services/
    │   ├── askService.ts   # AskService interface + StubAskService
    │   └── index.ts        # exports the active service (swap point for Plan 3)
    ├── stores/
    │   └── catalogue.ts    # useCatalogueStore (Pinia)
    ├── router/
    │   └── index.ts        # / and /title/:id
    ├── components/         # PosterCard, FilterBar, AskBar, ThreadBreadcrumb,
    │                       # RefineChips, AnswerContext, PosterGrid, ShimmerGrid,
    │                       # ServicePill, StarRating, AppHeader…
    └── views/
        ├── BrowseView.vue
        └── DetailView.vue
```

---

## 4. Data layer

### Types (`types.ts`) — mirror of backend `TitleDto`
```ts
export type ServiceKey = 'plex' | 'disney' | 'crunchyroll'
export type TitleKind  = 'movie' | 'series'

export interface Title {
  id: number
  imdbId: string | null
  title: string
  year: number
  services: ServiceKey[]      // multi-service
  type: TitleKind
  genres: string[]
  imdb: number | null         // nullable
  len: string                 // "164 min" | "28 eps"
  desc: string
  cast: string[]
  watched: boolean
  rating: number | null       // 1..5
}
```

### Client (`api/client.ts`)
- `getCatalogue(): Promise<Title[]>` — `GET /api/catalogue`, JSON, typed.
- Thin `fetch` wrapper; throws on non-2xx with a typed error the store surfaces
  as a load-error state (the grid shows a friendly message, not a blank screen).
- Dev: Vite proxies `/api` to `127.0.0.1:8080`, so no base-URL config in dev.
  Prod: same-origin (backend serves the SPA), so relative `/api` works unchanged.

---

## 5. State (`stores/catalogue.ts`, Pinia)

State:
- `catalogue: Title[]`, `status: 'idle'|'loading'|'ready'|'error'`
- Filters: `query`, `service: 'all'|ServiceKey`, `type: 'all'|TitleKind`,
  `genre: 'all'|string`, `sort: 'trending'|'rating'|'year'|'az'`
- Answer: `active`, `resultIds: number[]`, `line`, `sub`,
  `thread: { label, line, sub, ids }[]`, `resolving: boolean`
- User data: `watched: Record<number, boolean>`, `ratings: Record<number, 1..5>`
  (seeded from the catalogue response; mutated locally; **persisted in Plan 5**)

Getters:
- `visibleTitles` — base set (`answer.active ? resultIds : catalogue`) → apply
  `query` (title substring) / `service` (array `includes`) / `type` / `genre` → sort.
- `similar(id)` — other titles sharing ≥1 genre, ranked by shared-count then
  `imdb` (nulls last), top 5.
- `genres` — unique sorted genre list for the filter `<select>`.

Actions: `load()`, filter setters, `submitAsk(query)`, `refine(kind)`,
`moreLike(title)`, `stepThread(index)`, `clearThread()`, `toggleWatched(id)`,
`setRating(id, n)`. The ask actions set `resolving=true`, await the active
`AskService`, push a thread step, and set `resolving=false` — so the shimmer is
wired to real pending state.

---

## 6. The `askService` seam (key abstraction)

```ts
export interface AskResult { line: string; sub: string; ids: number[] }

export interface AskService {
  ask(query: string, base: Title[]): Promise<AskResult>
  refine(kind: 'lighter' | 'shorter' | 'surprise', current: Title[]): Promise<AskResult>
  similar(title: Title, all: Title[]): AskResult
}
```

- **Plan 2:** `StubAskService` implements the prototype's rule-based logic —
  curated sets for suggested prompts + keyword fallback over the catalogue;
  `lighter` keeps Comedy/Animation/Adventure/Romance/Musical; `shorter` sorts by
  length asc; `surprise` picks random `imdb ≥ 8`; `similar` by shared-genre.
  Returns resolve after a short awaited tick so the shimmer is exercised.
- **Plan 3:** `ApiAskService` calls `POST /api/ask`, constrained server-side to
  titles in the catalogue. Swap is a one-line change in `services/index.ts`.

---

## 7. Screens & components

### Header (`AppHeader`)
Fixed 58px; wordmark `cue` + amber dot + mono `SELF-HOSTED LIBRARY` eyebrow;
centered title-substring **search** input (filters the grid live); 32px avatar.
No header "Ask" button (Integrated mode keeps the ask in the canvas).

### Browse (`BrowseView`)
Top→bottom: sticky **AskBar** (+ **ThreadBreadcrumb** when a thread is active) →
**FilterBar** (Service segmented, Type segmented, Genre `<select>`, Sort
`<select>`, right-aligned result count) → **AnswerContext** (when an answer is
active: ✦ + answer line + sub-line + **RefineChips**) → **PosterGrid**
(`auto-fill, minmax(158px, 1fr)`, gap 22×18). While resolving, the grid swaps to
**ShimmerGrid**. Empty states distinguish plain-library-empty vs
over-filtered-answer copy.

### PosterCard
2/3 poster (placeholder gradient + monogram until real art), title, mono meta
(year · type), service dots, watched ✓ badge, per-card **✦ "find similar"**
(→ `moreLike`). Hover lift `translateY(-4px)` 160ms. Click → `/title/:id`.

### Detail (`DetailView`)
Backdrop band (360px, placeholder gradient + scrims) · Back button (→ Browse,
filters/thread preserved) · left column (poster, **Mark as watched** toggle
[display/local only], **StarRating** well) · right column (service pill(s) +
IMDb pill, `h1` title, mono fact line, description, **Cast** chips, **Similar
titles available** mini-grid via `similar(id)`). Multiple service pills render
when a title spans services.

### Interactions
- Ask submit (Enter/Ask) → resolve → reshape grid in place (300ms fade), no nav.
- Chat + filters **compose** (filters apply within the answer set).
- Thread: each ask/refine pushes a step; clicking a step restores+truncates;
  `Library` clears to full catalogue.
- Keyboard: `/` focuses the ask bar (when not typing in a field); `Esc` clears
  the active answer+thread (or blurs the focused field). Global `keydown`.

---

## 8. Styling

`tokens.css` provides global CSS custom properties; components use **scoped
styles** referencing them. `tokens.ts` supplies logic-side values and the
`posterPlaceholder()` / `monogram()` / `posterHue()` helpers. Fonts are
self-hosted via `@font-face`. No Tailwind/UI library. Preserve the design
principles: minimal chrome, content-forward, single amber accent, monospace
strictly for metadata/labels, generous poster sizing, no gradients beyond the
specified scrims/surfaces.

---

## 9. Testing (Vitest + Vue Test Utils)

- **Store:** `visibleTitles` composition (answer base → each filter → each sort),
  `similar` ranking + null-imdb handling, thread step/clear, `query` substring.
- **StubAskService:** keyword fallback, each refine kind, `similar`.
- **Components:** PosterCard (placeholder, watched badge, ✦ emits), FilterBar
  (segmented active states, genre/sort options), AskBar (answer-active states),
  empty-state copy selection.

No Playwright/e2e in Plan 2 — the Docker smoke CI already verifies the image
serves up. (A full e2e pass is a later consideration once the ask engine exists.)

---

## 10. Docker / build integration

Uncomment the Dockerfile's `frontend` build stage and the
`COPY --from=frontend /app/frontend/dist ./frontend/dist` line so the production
image bundles the real SPA (`STATIC_DIR=/app/frontend/dist` already set). The
existing `docker-smoke` CI then exercises the full image end-to-end.

---

## 11. Out of scope (Plan 2) / deferred

- **Real ask engine** — Plan 3 (`POST /api/ask`, embeddings + Claude).
- **Catalogue sync** — Plan 4 (Plex + Movie-of-the-Night).
- **Ratings/watched writes + persistence** — Plan 5 (endpoints + frontend wiring).
- Real poster artwork (placeholder generator until then).
- Auth (single-user, bound to `127.0.0.1`; middleware slot reserved).
- Mobile/responsive (design is explicitly desktop-only).

---

## 12. Open items to confirm during planning

- **Poster art source:** confirmed deferred — placeholder generator only in Plan 2.
- **`actions/checkout` bump:** the smoke CI emits a Node-20 deprecation; bump
  `@v4 → @v5` opportunistically (not blocking Plan 2).
- **Phase boundaries:** the implementation plan will finalize task granularity
  within the three phases (Foundation+Browse → Detail → Ask).
