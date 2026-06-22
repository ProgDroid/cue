# cue Frontend (Plan 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the cue frontend — a Vue 3 + TS + Pinia SPA that recreates the `design_handoff_cue/` design, served by the existing Rust backend, with live Browse + Detail and the integrated-ask UI behind a swappable stub.

**Architecture:** Vite-built Vue 3 (`<script setup>`) app in `frontend/`. A Pinia store holds the catalogue, filters, and ask/thread state; Vue Router handles `/` (Browse) and `/title/:id` (Detail). Data comes live from `GET /api/catalogue` typed against the real `TitleDto`. The ask experience calls an `AskService` interface whose Plan-2 implementation is a client-side stub (the prototype's rules); Plan 3 swaps in an API-backed implementation at one seam.

**Tech Stack:** Vue 3.5+, TypeScript, Vite, Pinia, Vue Router, Vitest + @vue/test-utils + jsdom. Bespoke styling from the handoff tokens (no Tailwind/UI library). Self-hosted fonts.

## Global Constraints

- **Design source of truth:** `design_handoff_cue/cue.dc.html` (markup + exact CSS), `design_handoff_cue/tokens.css` (CSS custom properties), `design_handoff_cue/tokens.ts` (logic tokens + `posterPlaceholder`/`monogram`/`posterHue` helpers). Recreate **pixel-accurately**. Ship **only the "Integrated" concept** — drop the bottom switcher and the Unified/Side-panel layouts.
- **Data contract = real backend `TitleDto`:** `services: ServiceKey[]` (array), `imdb: number | null`, plus `imdbId`, `watched`, `rating` present in the response. Service keys are `'plex' | 'disney' | 'crunchyroll'` (already match `tokens.ts`).
- **User-data writes are Plan 5:** `watched`/`rating` render from the response and may toggle locally for visual feedback, but there is **no** persistence endpoint yet — do not claim persistence.
- **Ask engine is Plan 3:** all ask logic goes through the `AskService` interface; Plan 2 ships `StubAskService` only. The swap point is `src/services/index.ts`.
- **Desktop-only**, mouse-navigated, dark theme. No responsive/mobile work.
- **Directory:** all frontend code under `frontend/`. The dev server proxies `/api` → `http://127.0.0.1:8080`; production is same-origin (backend serves `frontend/dist`).
- **Backend conventions** (`CLAUDE.md`) are unaffected — no Rust changes except enabling the Dockerfile frontend stage (Task 16).
- **Commits:** conventional commits; make commits via the Bash tool (PowerShell prepends a BOM to commit subjects on this machine).

---

## File Structure

```
frontend/
├── index.html                     # title "cue", font preconnect/links removed (self-hosted)
├── package.json
├── vite.config.ts                 # @vitejs/plugin-vue, test (jsdom), server.proxy /api
├── tsconfig.json / tsconfig.node.json
├── vitest.setup.ts                # (optional) global test setup
└── src/
    ├── main.ts                    # createApp + Pinia + Router, import design CSS
    ├── App.vue                    # <AppHeader/> + <RouterView/>
    ├── design/
    │   ├── tokens.ts              # copied from handoff
    │   ├── tokens.css             # copied from handoff (global :root vars)
    │   └── fonts.css              # @font-face, self-hosted
    ├── assets/fonts/              # Hanken Grotesk + JetBrains Mono woff2
    ├── types.ts                   # Title, ServiceKey, TitleKind, AskResult, ThreadStep
    ├── api/client.ts              # getCatalogue()
    ├── services/
    │   ├── askService.ts          # AskService interface + StubAskService
    │   └── index.ts               # export const askService = new StubAskService()  (swap point)
    ├── stores/catalogue.ts        # useCatalogueStore
    ├── router/index.ts            # routes: / , /title/:id
    ├── components/
    │   ├── AppHeader.vue
    │   ├── PosterCard.vue
    │   ├── FilterBar.vue
    │   ├── PosterGrid.vue
    │   ├── ShimmerGrid.vue
    │   ├── AskBar.vue
    │   ├── ThreadBreadcrumb.vue
    │   ├── AnswerContext.vue
    │   ├── RefineChips.vue
    │   ├── ServicePill.vue
    │   └── StarRating.vue
    └── views/
        ├── BrowseView.vue
        └── DetailView.vue
```

---

## Phase 1 — Foundation + Browse

### Task 1: Scaffold the Vue app

**Files:**
- Create: `frontend/` (whole Vite project)
- Modify: `frontend/vite.config.ts`, `frontend/src/main.ts`, `frontend/index.html`
- Create: `frontend/src/design/tokens.ts`, `frontend/src/design/tokens.css`, `frontend/src/design/fonts.css`

**Interfaces:**
- Produces: a booting Vite app with Pinia + Router installed and design tokens imported; `npm run build` and `npm run test` both succeed.

- [ ] **Step 1: Scaffold with the stable Vite vue-ts template (non-interactive)**

Run from repo root (`G:/rustDev/cue`):
```bash
npm create vite@latest frontend -- --template vue-ts
cd frontend && npm install
```
If the CLI prompts for a Vite variant, accept the default (non-rolldown) Vue + TypeScript.

- [ ] **Step 2: Install runtime + test deps**

```bash
cd frontend
npm install pinia vue-router
npm install -D vitest @vue/test-utils jsdom
```

- [ ] **Step 3: Configure Vite (plugin already present) — add test env + dev proxy**

Replace `frontend/vite.config.ts` with:
```ts
/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080', changeOrigin: true },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
  },
})
```

Add a test script to `frontend/package.json` `"scripts"`:
```json
"test": "vitest run",
"test:watch": "vitest"
```

- [ ] **Step 4: Copy design tokens + add fonts**

```bash
cp ../design_handoff_cue/tokens.ts src/design/tokens.ts
cp ../design_handoff_cue/tokens.css src/design/tokens.css
```
Create `src/design/fonts.css` with `@font-face` rules for Hanken Grotesk (weights 400–800) and JetBrains Mono (400–600), `font-display: swap`, pointing at `@/assets/fonts/*.woff2`. Download the woff2 files into `src/assets/fonts/` (Google Fonts → self-host; or `npm install @fontsource/hanken-grotesk @fontsource/jetbrains-mono` and import those CSS files instead of hand-writing `fonts.css`). Set `body { font-family: var(--font-ui); background: var(--bg-app); color: var(--text-primary); }` in a small global block (top of `tokens.css` is fine, or `main.ts` import order).

- [ ] **Step 5: Wire `main.ts`**

Replace `src/main.ts`:
```ts
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import { router } from './router'
import './design/fonts.css'
import './design/tokens.css'

createApp(App).use(createPinia()).use(router).mount('#app')
```
Create a minimal `src/router/index.ts` (filled out in Task 8) exporting `router` with a single `/` route rendering `BrowseView` (stub `BrowseView.vue` with `<template><div>browse</div></template>` for now). Replace `src/App.vue` with `<template><RouterView /></template>` + `<script setup lang="ts"></script>`.

- [ ] **Step 6: Smoke test — app mounts**

Create `src/__tests__/smoke.test.ts`:
```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import App from '../App.vue'
import { router } from '../router'

describe('App', () => {
  it('mounts without error', async () => {
    const wrapper = mount(App, { global: { plugins: [router] } })
    expect(wrapper.exists()).toBe(true)
  })
})
```

- [ ] **Step 7: Run build + test**

```bash
npm run build
npm run test
```
Expected: build succeeds (emits `dist/`), test passes.

- [ ] **Step 8: Update `.gitignore` + commit**

Ensure repo `.gitignore` covers `frontend/node_modules` and `frontend/dist` (root `.gitignore` already has `/frontend/dist` and `node_modules`). Then:
```bash
git add frontend .gitignore
git commit -m "feat(frontend): scaffold Vue 3 + TS + Pinia + Router app"
```

---

### Task 2: Types + API client

**Files:**
- Create: `frontend/src/types.ts`
- Create: `frontend/src/api/client.ts`
- Test: `frontend/src/api/__tests__/client.test.ts`

**Interfaces:**
- Produces: `Title`, `ServiceKey`, `TitleKind`, `AskResult`, `ThreadStep` types; `getCatalogue(): Promise<Title[]>`.

- [ ] **Step 1: Write the types**

`src/types.ts`:
```ts
export type ServiceKey = 'plex' | 'disney' | 'crunchyroll'
export type TitleKind = 'movie' | 'series'

export interface Title {
  id: number
  imdbId: string | null
  title: string
  year: number
  services: ServiceKey[]
  type: TitleKind
  genres: string[]
  imdb: number | null
  len: string
  desc: string
  cast: string[]
  watched: boolean
  rating: number | null
}

export interface AskResult {
  line: string
  sub: string
  ids: number[]
}

export interface ThreadStep {
  label: string
  line: string
  sub: string
  ids: number[]
}
```

- [ ] **Step 2: Write the failing client test**

`src/api/__tests__/client.test.ts`:
```ts
import { describe, it, expect, vi, afterEach } from 'vitest'
import { getCatalogue } from '../client'

const sample = [{
  id: 1, imdbId: 'tt1', title: 'X', year: 2020, services: ['plex'],
  type: 'movie', genres: ['Comedy'], imdb: 8.1, len: '90 min',
  desc: 'd', cast: ['A'], watched: false, rating: null,
}]

afterEach(() => vi.restoreAllMocks())

describe('getCatalogue', () => {
  it('returns parsed titles on 200', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true, status: 200, json: async () => sample,
    }))
    const titles = await getCatalogue()
    expect(titles).toHaveLength(1)
    expect(titles[0].services).toEqual(['plex'])
  })

  it('throws on non-2xx', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 500, json: async () => ({}) }))
    await expect(getCatalogue()).rejects.toThrow(/catalogue/i)
  })
})
```

- [ ] **Step 3: Run test to verify it fails**

Run: `npm run test -- client`
Expected: FAIL (`getCatalogue` not found).

- [ ] **Step 4: Implement the client**

`src/api/client.ts`:
```ts
import type { Title } from '@/types'

export async function getCatalogue(): Promise<Title[]> {
  const res = await fetch('/api/catalogue')
  if (!res.ok) {
    throw new Error(`Failed to load catalogue (HTTP ${res.status})`)
  }
  return (await res.json()) as Title[]
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `npm run test -- client`
Expected: PASS (both cases).

- [ ] **Step 6: Commit**

```bash
git add src/types.ts src/api
git commit -m "feat(frontend): add Title types and catalogue API client"
```

---

### Task 3: Catalogue store — load, filters, visibleTitles, genres

**Files:**
- Create: `frontend/src/stores/catalogue.ts`
- Test: `frontend/src/stores/__tests__/catalogue.test.ts`

**Interfaces:**
- Consumes: `getCatalogue` (Task 2), `Title` (Task 2).
- Produces: `useCatalogueStore` with state `catalogue, status, query, service, type, genre, sort` and getters `visibleTitles`, `genres`. Filter setters: `setQuery/setService/setType/setGenre/setSort`. Action `load()`. (Ask state added in Task 12.)

- [ ] **Step 1: Write failing store tests**

`src/stores/__tests__/catalogue.test.ts`:
```ts
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '../catalogue'
import type { Title } from '@/types'

function t(p: Partial<Title>): Title {
  return {
    id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], imdb: null, len: '90 min', desc: '',
    cast: [], watched: false, rating: null, ...p,
  }
}

const fixtures: Title[] = [
  t({ id: 1, title: 'Frieren', type: 'series', services: ['crunchyroll'], genres: ['Animation', 'Adventure'], imdb: 9.0, year: 2023 }),
  t({ id: 2, title: 'Coco', type: 'movie', services: ['disney'], genres: ['Animation', 'Musical'], imdb: 8.4, year: 2017 }),
  t({ id: 3, title: 'Alien', type: 'movie', services: ['plex', 'disney'], genres: ['Horror'], imdb: 8.5, year: 1979 }),
]

beforeEach(() => {
  setActivePinia(createPinia())
  vi.restoreAllMocks()
})

describe('catalogue store', () => {
  it('load() populates catalogue and sets status ready', async () => {
    const s = useCatalogueStore()
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue(fixtures)
    await s.load()
    expect(s.status).toBe('ready')
    expect(s.catalogue).toHaveLength(3)
  })

  it('visibleTitles filters by service membership (array includes)', async () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures
    s.setService('disney')
    expect(s.visibleTitles.map(x => x.id).sort()).toEqual([2, 3])
  })

  it('visibleTitles filters by type and query substring (case-insensitive)', () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures
    s.setType('movie'); s.setQuery('ali')
    expect(s.visibleTitles.map(x => x.id)).toEqual([3])
  })

  it('sort=az orders by title; sort=year orders desc; sort=rating orders desc with nulls last', () => {
    const s = useCatalogueStore()
    s.catalogue = [...fixtures, t({ id: 4, title: 'Aaa', imdb: null, year: 1990 })]
    s.setSort('az')
    expect(s.visibleTitles.map(x => x.title)[0]).toBe('Aaa')
    s.setSort('year')
    expect(s.visibleTitles.map(x => x.id)[0]).toBe(1) // 2023
    s.setSort('rating')
    expect(s.visibleTitles.map(x => x.id).at(-1)).toBe(4) // null imdb last
  })

  it('genres getter returns unique sorted genres', () => {
    const s = useCatalogueStore()
    s.catalogue = fixtures
    expect(s.genres).toEqual(['Adventure', 'Animation', 'Horror', 'Musical'])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- catalogue`
Expected: FAIL (`useCatalogueStore` not found).

- [ ] **Step 3: Implement the store (filters + getters)**

`src/stores/catalogue.ts`:
```ts
import { defineStore } from 'pinia'
import type { ServiceKey, Title, TitleKind } from '@/types'
import { getCatalogue } from '@/api/client'

type Status = 'idle' | 'loading' | 'ready' | 'error'
type ServiceFilter = 'all' | ServiceKey
type TypeFilter = 'all' | TitleKind
type SortKey = 'trending' | 'rating' | 'year' | 'az'

interface State {
  catalogue: Title[]
  status: Status
  error: string | null
  query: string
  service: ServiceFilter
  type: TypeFilter
  genre: 'all' | string
  sort: SortKey
}

export const useCatalogueStore = defineStore('catalogue', {
  state: (): State => ({
    catalogue: [],
    status: 'idle',
    error: null,
    query: '',
    service: 'all',
    type: 'all',
    genre: 'all',
    sort: 'trending',
  }),

  getters: {
    genres(state): string[] {
      const set = new Set<string>()
      for (const t of state.catalogue) for (const g of t.genres) set.add(g)
      return [...set].sort()
    },

    visibleTitles(state): Title[] {
      let out = state.catalogue.slice() // base set (answer set overrides this in Task 12)
      const q = state.query.trim().toLowerCase()
      if (q) out = out.filter(t => t.title.toLowerCase().includes(q))
      if (state.service !== 'all') out = out.filter(t => t.services.includes(state.service as ServiceKey))
      if (state.type !== 'all') out = out.filter(t => t.type === state.type)
      if (state.genre !== 'all') out = out.filter(t => t.genres.includes(state.genre))

      const byRating = (a: Title, b: Title) =>
        (b.imdb ?? -Infinity) - (a.imdb ?? -Infinity)
      switch (state.sort) {
        case 'az': out.sort((a, b) => a.title.localeCompare(b.title)); break
        case 'year': out.sort((a, b) => b.year - a.year); break
        case 'rating': out.sort(byRating); break
        case 'trending': /* keep base order */ break
      }
      return out
    },
  },

  actions: {
    setQuery(v: string) { this.query = v },
    setService(v: ServiceFilter) { this.service = v },
    setType(v: TypeFilter) { this.type = v },
    setGenre(v: 'all' | string) { this.genre = v },
    setSort(v: SortKey) { this.sort = v },

    async load() {
      this.status = 'loading'
      this.error = null
      try {
        this.catalogue = await getCatalogue()
        this.status = 'ready'
      } catch (e) {
        this.error = e instanceof Error ? e.message : 'Failed to load catalogue'
        this.status = 'error'
      }
    },
  },
})
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- catalogue`
Expected: PASS (all 5).

- [ ] **Step 5: Commit**

```bash
git add src/stores
git commit -m "feat(frontend): catalogue store with filters, sort, genres"
```

---

### Task 4: `similar(id)` getter + user-data state

**Files:**
- Modify: `frontend/src/stores/catalogue.ts`
- Test: `frontend/src/stores/__tests__/similar.test.ts`

**Interfaces:**
- Produces: getter `similar(id: number): Title[]` (others sharing ≥1 genre, ranked by shared-count then `imdb` desc nulls-last, top 5); state `watched: Record<number, boolean>`, `ratings: Record<number, number>` seeded from the catalogue response; actions `toggleWatched(id)`, `setRating(id, n)`; getters `isWatched(id)`, `ratingOf(id)`.

- [ ] **Step 1: Write failing test**

`src/stores/__tests__/similar.test.ts`:
```ts
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '../catalogue'
import type { Title } from '@/types'

function t(p: Partial<Title>): Title {
  return { id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], imdb: null, len: '90 min', desc: '',
    cast: [], watched: false, rating: null, ...p }
}

beforeEach(() => setActivePinia(createPinia()))

describe('similar + user data', () => {
  const cat: Title[] = [
    t({ id: 1, genres: ['Animation', 'Adventure'], imdb: 9 }),
    t({ id: 2, genres: ['Animation', 'Adventure'], imdb: 8 }), // 2 shared
    t({ id: 3, genres: ['Animation'], imdb: 8.5 }),            // 1 shared
    t({ id: 4, genres: ['Horror'], imdb: 9.9 }),               // 0 shared
  ]

  it('ranks by shared-genre count then imdb desc, excludes self, top 5', () => {
    const s = useCatalogueStore(); s.catalogue = cat
    expect(s.similar(1).map(x => x.id)).toEqual([2, 3])
  })

  it('toggleWatched and setRating update derived getters', () => {
    const s = useCatalogueStore(); s.catalogue = cat
    s.toggleWatched(1); expect(s.isWatched(1)).toBe(true)
    s.setRating(1, 4); expect(s.ratingOf(1)).toBe(4)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- similar`
Expected: FAIL (`similar`/`isWatched` undefined).

- [ ] **Step 3: Implement**

Add to `State`: `watched: Record<number, boolean>` and `ratings: Record<number, number>` (init `{}`). In `load()`, after assigning `this.catalogue`, seed them:
```ts
for (const t of this.catalogue) {
  if (t.watched) this.watched[t.id] = true
  if (t.rating != null) this.ratings[t.id] = t.rating
}
```
Add getters:
```ts
isWatched: (s) => (id: number) => !!s.watched[id],
ratingOf: (s) => (id: number) => s.ratings[id] ?? null,
similar() {
  return (id: number): Title[] => {
    const self = this.catalogue.find(t => t.id === id)
    if (!self) return []
    const selfGenres = new Set(self.genres)
    return this.catalogue
      .filter(t => t.id !== id && t.genres.some(g => selfGenres.has(g)))
      .map(t => ({ t, shared: t.genres.filter(g => selfGenres.has(g)).length }))
      .sort((a, b) => b.shared - a.shared || (b.t.imdb ?? -Infinity) - (a.t.imdb ?? -Infinity))
      .slice(0, 5)
      .map(x => x.t)
  }
},
```
Add actions:
```ts
toggleWatched(id: number) { this.watched[id] = !this.watched[id] },
setRating(id: number, n: number) { this.ratings[id] = n },
```
> Note: `toggleWatched`/`setRating` mutate local state only — persistence is Plan 5.

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- similar`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/stores
git commit -m "feat(frontend): similar() getter and local watched/rating state"
```

---

### Task 5: PosterCard component

**Files:**
- Create: `frontend/src/components/PosterCard.vue`
- Create: `frontend/src/components/ServicePill.vue`
- Test: `frontend/src/components/__tests__/PosterCard.test.ts`

**Interfaces:**
- Consumes: `Title` (Task 2); `posterPlaceholder`, `monogram` from `@/design/tokens`.
- Produces: `<PosterCard :title="Title" :watched="boolean" />`, emits `select` (card click) and `find-similar` (✦ button). `<ServicePill :service="ServiceKey" />` renders dot + label from `tokens.services`.

- [ ] **Step 1: Write failing component test**

`src/components/__tests__/PosterCard.test.ts`:
```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import PosterCard from '../PosterCard.vue'
import type { Title } from '@/types'

const title: Title = {
  id: 7, imdbId: 'tt7', title: 'Frieren', year: 2023, services: ['crunchyroll'],
  type: 'series', genres: ['Animation'], imdb: 9.0, len: '28 eps',
  desc: '', cast: [], watched: false, rating: null,
}

describe('PosterCard', () => {
  it('renders title and a monogram placeholder (no <img> until real art)', () => {
    const w = mount(PosterCard, { props: { title, watched: false } })
    expect(w.text()).toContain('Frieren')
    expect(w.find('img').exists()).toBe(false)
  })

  it('emits select on card click and find-similar on ✦', async () => {
    const w = mount(PosterCard, { props: { title, watched: false } })
    await w.get('[data-test="card"]').trigger('click')
    expect(w.emitted('select')?.[0]).toEqual([7])
    await w.get('[data-test="find-similar"]').trigger('click')
    expect(w.emitted('find-similar')?.[0]).toEqual([title])
  })

  it('shows the watched badge when watched', () => {
    const w = mount(PosterCard, { props: { title, watched: true } })
    expect(w.find('[data-test="watched-badge"]').exists()).toBe(true)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- PosterCard`
Expected: FAIL (component missing).

- [ ] **Step 3: Implement PosterCard + ServicePill**

Port the poster-card markup and CSS from `design_handoff_cue/cue.dc.html` (the `.card` / poster / meta-row structure) and `tokens.css`. The `<script setup lang="ts">` contract:
```ts
import { computed } from 'vue'
import type { Title } from '@/types'
import { posterPlaceholder, monogram } from '@/design/tokens'

const props = defineProps<{ title: Title; watched: boolean }>()
const emit = defineEmits<{ select: [id: number]; 'find-similar': [t: Title] }>()
const ph = computed(() => posterPlaceholder(props.title.title))
const mono = computed(() => monogram(props.title.title))
```
Template requirements (recreate exact styles from the prototype):
- Root `[data-test="card"]` with `@click="emit('select', title.id)"`, hover lift `translateY(-4px)` 160ms `var(--ease)`.
- Poster block at aspect `2 / 3`, `background: ph.background`, the monogram `mono` rendered faint (use `ph.glyphColor`), motif `ph.motif`. **No `<img>`** (placeholder only this plan).
- Watched badge `[data-test="watched-badge"]` (amber `✓`, ~22px, bottom-right) shown `v-if="watched"`.
- Meta row: title (`var(--type-base)`), mono year·type, service dots (loop `title.services` → small dot using `tokens.services[s].dot`), and the per-card `[data-test="find-similar"]` ✦ button `@click.stop="emit('find-similar', title)"`.

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- PosterCard`
Expected: PASS (3 cases).

- [ ] **Step 5: Commit**

```bash
git add src/components/PosterCard.vue src/components/ServicePill.vue src/components/__tests__/PosterCard.test.ts
git commit -m "feat(frontend): PosterCard and ServicePill components"
```

---

### Task 6: AppHeader (wordmark + live search)

**Files:**
- Create: `frontend/src/components/AppHeader.vue`
- Modify: `frontend/src/App.vue` (render `<AppHeader/>` above `<RouterView/>`)
- Test: `frontend/src/components/__tests__/AppHeader.test.ts`

**Interfaces:**
- Consumes: `useCatalogueStore` (`query`, `setQuery`).
- Produces: header with wordmark `cue` + amber dot + `SELF-HOSTED LIBRARY` eyebrow, centered search input two-way bound to `store.query`, avatar `JD`.

- [ ] **Step 1: Write failing test**

`src/components/__tests__/AppHeader.test.ts`:
```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import AppHeader from '../AppHeader.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))

describe('AppHeader', () => {
  it('typing in search updates store.query', async () => {
    const w = mount(AppHeader)
    const s = useCatalogueStore()
    await w.get('input[type="search"], input[data-test="search"]').setValue('frieren')
    expect(s.query).toBe('frieren')
  })

  it('renders the wordmark', () => {
    const w = mount(AppHeader)
    expect(w.text()).toContain('cue')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- AppHeader`
Expected: FAIL.

- [ ] **Step 3: Implement**

Port the header markup/CSS from the prototype (fixed 58px, blur bg, wordmark + dot + eyebrow, centered search `max-width:440px`, avatar). Bind the input:
```ts
import { computed } from 'vue'
import { useCatalogueStore } from '@/stores/catalogue'
const store = useCatalogueStore()
const query = computed({ get: () => store.query, set: v => store.setQuery(v) })
```
Input: `<input data-test="search" type="search" v-model="query" placeholder="Search titles" />` with the leading `⌕` glyph. No header "Ask" button (Integrated mode).

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- AppHeader`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/AppHeader.vue src/App.vue src/components/__tests__/AppHeader.test.ts
git commit -m "feat(frontend): AppHeader with live title search"
```

---

### Task 7: FilterBar

**Files:**
- Create: `frontend/src/components/FilterBar.vue`
- Test: `frontend/src/components/__tests__/FilterBar.test.ts`

**Interfaces:**
- Consumes: `useCatalogueStore` (`service/type/genre/sort` + setters, `genres`, `visibleTitles.length`).
- Produces: Service segmented (`All·Plex·Disney+·Crunchyroll`), Type segmented (`All·Movies·Series`), Genre `<select>`, Sort `<select>`, right-aligned result count.

- [ ] **Step 1: Write failing test**

`src/components/__tests__/FilterBar.test.ts`:
```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import FilterBar from '../FilterBar.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import type { Title } from '@/types'

beforeEach(() => setActivePinia(createPinia()))

describe('FilterBar', () => {
  it('clicking Plex sets store.service and marks the button active', async () => {
    const w = mount(FilterBar)
    const s = useCatalogueStore()
    await w.get('[data-test="service-plex"]').trigger('click')
    expect(s.service).toBe('plex')
    expect(w.get('[data-test="service-plex"]').classes()).toContain('active')
  })

  it('shows the result count from visibleTitles', () => {
    const s = useCatalogueStore()
    s.catalogue = [{ id: 1 } as Title, { id: 2 } as Title]
    const w = mount(FilterBar)
    expect(w.get('[data-test="count"]').text()).toMatch(/2 titles/)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- FilterBar`
Expected: FAIL.

- [ ] **Step 3: Implement**

Port the filter-bar markup/CSS from the prototype. Segmented buttons set the store value; add class `active` when selected. Service buttons (non-All) show their `tokens.services[key].dot`. Genre `<select>` options = `['all', ...store.genres]`; Sort options = `Trending|Top rated|Newest|A–Z` mapped to `trending|rating|year|az`. Result count `[data-test="count"]` = `` `${store.visibleTitles.length} titles` ``. Give each service button `data-test="service-<key>"`.

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- FilterBar`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/FilterBar.vue src/components/__tests__/FilterBar.test.ts
git commit -m "feat(frontend): FilterBar (service/type/genre/sort + count)"
```

---

### Task 8: BrowseView assembly + PosterGrid + empty states + routing

**Files:**
- Create: `frontend/src/components/PosterGrid.vue`
- Create/replace: `frontend/src/views/BrowseView.vue`
- Modify: `frontend/src/router/index.ts` (add `/title/:id`)
- Test: `frontend/src/views/__tests__/BrowseView.test.ts`

**Interfaces:**
- Consumes: `useCatalogueStore`, `PosterCard`, `FilterBar`. Router pushes `/title/:id` on card `select`.
- Produces: `BrowseView` calling `store.load()` on mount, rendering FilterBar + grid, empty-state copy (plain-library-empty now; over-filtered-answer copy added in Task 14).

- [ ] **Step 1: Write failing test**

`src/views/__tests__/BrowseView.test.ts`:
```ts
import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import BrowseView from '../BrowseView.vue'
import type { Title } from '@/types'

const routes = [
  { path: '/', component: BrowseView },
  { path: '/title/:id', component: { template: '<div>detail</div>' } },
]

beforeEach(() => setActivePinia(createPinia()))

function makeRouter() { return createRouter({ history: createMemoryHistory(), routes }) }

describe('BrowseView', () => {
  it('loads catalogue on mount and renders cards', async () => {
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue([
      { id: 1, title: 'A' } as Title, { id: 2, title: 'B' } as Title,
    ].map(p => ({ services: ['plex'], type: 'movie', genres: [], imdb: null, year: 2000, len: '', desc: '', cast: [], watched: false, rating: null, imdbId: null, ...p } as Title)))
    const router = makeRouter(); router.push('/'); await router.isReady()
    const w = mount(BrowseView, { global: { plugins: [router] } })
    await flushPromises()
    expect(w.findAll('[data-test="card"]').length).toBe(2)
  })

  it('shows plain empty-state copy when no titles match', async () => {
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue([])
    const router = makeRouter(); router.push('/'); await router.isReady()
    const w = mount(BrowseView, { global: { plugins: [router] } })
    await flushPromises()
    expect(w.text()).toContain('Nothing in your library matches those filters')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- BrowseView`
Expected: FAIL.

- [ ] **Step 3: Implement PosterGrid, BrowseView, route**

`PosterGrid.vue`: a CSS grid (`repeat(auto-fill, minmax(158px, 1fr))`, gap `22px 18px`) that takes `:titles="Title[]"`, renders a `PosterCard` per title (pass `:watched="store.isWatched(t.id)"`), and re-emits `select`/`find-similar`.

`BrowseView.vue`:
```ts
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useCatalogueStore } from '@/stores/catalogue'
const store = useCatalogueStore()
const router = useRouter()
onMounted(() => { if (store.status === 'idle') store.load() })
function openDetail(id: number) { router.push(`/title/${id}`) }
// find-similar wired in Task 15
```
Template: `<FilterBar/>` then either `<PosterGrid :titles="store.visibleTitles" @select="openDetail" />` or, when `store.visibleTitles.length === 0`, an empty-state block. Empty copy (Task 14 makes this conditional on answer-active):
`"Nothing in your library matches those filters."`
Add the `/title/:id` route pointing at `DetailView` (created Task 9 — for now point at a stub or create `DetailView.vue` with `<template><div>detail</div></template>`).

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- BrowseView`
Expected: PASS.

- [ ] **Step 5: Manual check against the running backend**

```bash
# Terminal A (repo root): cargo run    # backend on 127.0.0.1:8080 with seeded catalogue
# Terminal B: cd frontend && npm run dev
```
Open the dev URL; confirm 28 posters render, search/service/type/genre/sort all filter live, and clicking a card navigates to `/title/:id`.

- [ ] **Step 6: Commit**

```bash
git add src/components/PosterGrid.vue src/views/BrowseView.vue src/router/index.ts src/views/__tests__/BrowseView.test.ts
git commit -m "feat(frontend): Browse view with live grid, filters, empty state, routing"
```

---

## Phase 2 — Detail

### Task 9: DetailView (backdrop, poster, facts, cast, pills)

**Files:**
- Create/replace: `frontend/src/views/DetailView.vue`
- Test: `frontend/src/views/__tests__/DetailView.test.ts`

**Interfaces:**
- Consumes: `useCatalogueStore` (`catalogue`, `similar`, `isWatched`, `ratingOf`), route param `id`, `ServicePill`, `posterPlaceholder`/`monogram`.
- Produces: Detail screen for the routed title; Back button navigates to `/` (filters/thread preserved because state lives in the store).

- [ ] **Step 1: Write failing test**

`src/views/__tests__/DetailView.test.ts`:
```ts
import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import DetailView from '../DetailView.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import type { Title } from '@/types'

const title: Title = {
  id: 5, imdbId: 'tt5', title: 'Coco', year: 2017, services: ['disney'],
  type: 'movie', genres: ['Animation', 'Musical'], imdb: 8.4, len: '105 min',
  desc: 'A boy and music.', cast: ['Anthony Gonzalez'], watched: false, rating: null,
}

beforeEach(() => setActivePinia(createPinia()))

describe('DetailView', () => {
  it('renders the routed title facts and cast', async () => {
    const router = createRouter({ history: createMemoryHistory(), routes: [
      { path: '/', component: { template: '<div>home</div>' } },
      { path: '/title/:id', component: DetailView },
    ] })
    const s = useCatalogueStore(); s.catalogue = [title]
    router.push('/title/5'); await router.isReady()
    const w = mount(DetailView, { global: { plugins: [router] } })
    await flushPromises()
    expect(w.text()).toContain('Coco')
    expect(w.text()).toContain('Anthony Gonzalez')
    expect(w.text()).toContain('2017')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- DetailView`
Expected: FAIL.

- [ ] **Step 3: Implement**

Port the Detail layout/CSS from the prototype (backdrop band 360px + scrims, `max-width:1080px` body pulled up `-180px`, left poster column / right info column). Logic:
```ts
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useCatalogueStore } from '@/stores/catalogue'
const route = useRoute(); const router = useRouter()
const store = useCatalogueStore()
const id = computed(() => Number(route.params.id))
const title = computed(() => store.catalogue.find(t => t.id === id.value) ?? null)
function back() { router.push('/') }
```
Render: Back button (`← Library`, `@click="back"`); badge row = one `ServicePill` per `title.services` + amber IMDb pill `★ {imdb} IMDb` (omit if `imdb == null`); `h1` title; mono fact line `{year} · {Movie|Series} · {len} · {genres}`; description; Cast chips; "Similar titles available" mini-grid (`store.similar(id)` → mini PosterCards, navigate on select). If `title` is null (catalogue not loaded yet on a deep link), call `store.load()` `onMounted` and show nothing until ready.

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- DetailView`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/views/DetailView.vue src/views/__tests__/DetailView.test.ts
git commit -m "feat(frontend): Detail view (backdrop, facts, cast, similar)"
```

---

### Task 10: StarRating + Mark-as-watched (local)

**Files:**
- Create: `frontend/src/components/StarRating.vue`
- Modify: `frontend/src/views/DetailView.vue` (add watched button + rating well)
- Test: `frontend/src/components/__tests__/StarRating.test.ts`

**Interfaces:**
- Consumes: `useCatalogueStore` (`toggleWatched`, `setRating`, `isWatched`, `ratingOf`).
- Produces: `<StarRating :value="number|null" @set="(n)=>..." />` with 5 clickable stars; DetailView watched toggle button.

- [ ] **Step 1: Write failing test**

`src/components/__tests__/StarRating.test.ts`:
```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import StarRating from '../StarRating.vue'

describe('StarRating', () => {
  it('renders 5 stars and emits set with the clicked index (1-based)', async () => {
    const w = mount(StarRating, { props: { value: null } })
    const stars = w.findAll('[data-test="star"]')
    expect(stars).toHaveLength(5)
    await stars[3].trigger('click')
    expect(w.emitted('set')?.[0]).toEqual([4])
  })

  it('marks stars up to value as filled', () => {
    const w = mount(StarRating, { props: { value: 3 } })
    const filled = w.findAll('[data-test="star"].filled')
    expect(filled).toHaveLength(3)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- StarRating`
Expected: FAIL.

- [ ] **Step 3: Implement**

`StarRating.vue`: props `{ value: number | null }`, emit `set: [n: number]`. Render 5 `[data-test="star"]` `★`; add class `filled` for index ≤ value (filled `#f5c518`, empty `#3a3f4a`, 24px). In DetailView, add the **Mark as watched** button (default amber fill / watched amber-tint per prototype, `@click="store.toggleWatched(id)"`, label/state from `store.isWatched(id)`) and a "Your rating" well wrapping `<StarRating :value="store.ratingOf(id)" @set="n => store.setRating(id, n)" />`. Add a one-line comment: writes are local until Plan 5.

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- StarRating`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/StarRating.vue src/views/DetailView.vue src/components/__tests__/StarRating.test.ts
git commit -m "feat(frontend): StarRating and local mark-as-watched"
```

---

## Phase 3 — Integrated ask

### Task 11: AskService interface + StubAskService

**Files:**
- Create: `frontend/src/services/askService.ts`
- Create: `frontend/src/services/index.ts`
- Test: `frontend/src/services/__tests__/stub.test.ts`

**Interfaces:**
- Consumes: `Title`, `AskResult` (Task 2).
- Produces: `interface AskService { ask(query, base): Promise<AskResult>; refine(kind, current): Promise<AskResult>; similar(title, all): AskResult }`; `class StubAskService implements AskService`; `export const askService: AskService` (the swap point).

- [ ] **Step 1: Write failing tests**

`src/services/__tests__/stub.test.ts`:
```ts
import { describe, it, expect } from 'vitest'
import { StubAskService } from '../askService'
import type { Title } from '@/types'

function t(p: Partial<Title>): Title {
  return { id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], imdb: null, len: '90 min', desc: '',
    cast: [], watched: false, rating: null, ...p }
}
const cat: Title[] = [
  t({ id: 1, title: 'Frieren', genres: ['Animation', 'Adventure'], imdb: 9.0, len: '28 eps' }),
  t({ id: 2, title: 'Coco', genres: ['Animation', 'Musical'], imdb: 8.4, len: '105 min' }),
  t({ id: 3, title: 'Alien', genres: ['Horror'], imdb: 8.5, len: '117 min' }),
]
const svc = new StubAskService()

describe('StubAskService', () => {
  it('ask() keyword-matches over title/genre and returns ids + a line', async () => {
    const r = await svc.ask('animation', cat)
    expect(r.ids).toEqual(expect.arrayContaining([1, 2]))
    expect(r.ids).not.toContain(3)
    expect(r.line.length).toBeGreaterThan(0)
  })

  it("refine('lighter') keeps light genres", async () => {
    const r = await svc.refine('lighter', cat)
    expect(r.ids).toEqual(expect.arrayContaining([1, 2]))
    expect(r.ids).not.toContain(3)
  })

  it("refine('shorter') orders by ascending watch length", async () => {
    // lenMinutes: Coco 105 min -> 105, Alien 117 min -> 117, Frieren 28 eps -> 28*24=672
    const r = await svc.refine('shorter', cat)
    expect(r.ids).toEqual([2, 3, 1])
  })

  it('similar() ranks by shared genre', () => {
    const r = svc.similar(cat[0], cat)
    expect(r.ids).toEqual([2])
    expect(r.line).toContain('Frieren')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- stub`
Expected: FAIL.

- [ ] **Step 3: Implement**

`src/services/askService.ts`:
```ts
import type { AskResult, Title } from '@/types'

const LIGHT = new Set(['Comedy', 'Animation', 'Adventure', 'Romance', 'Musical'])

// "164 min" -> 164; "28 eps" -> eps*~24 so series sort sensibly among movies.
function lenMinutes(len: string): number {
  const n = parseInt(len, 10) || 0
  return /eps/i.test(len) ? n * 24 : n
}

export interface AskService {
  ask(query: string, base: Title[]): Promise<AskResult>
  refine(kind: 'lighter' | 'shorter' | 'surprise', current: Title[]): Promise<AskResult>
  similar(title: Title, all: Title[]): AskResult
}

export class StubAskService implements AskService {
  async ask(query: string, base: Title[]): Promise<AskResult> {
    await Promise.resolve() // keep the await seam so the shimmer is exercised
    const q = query.trim().toLowerCase()
    const hits = base.filter(t =>
      t.title.toLowerCase().includes(q) ||
      t.genres.some(g => g.toLowerCase().includes(q)) ||
      t.desc.toLowerCase().includes(q))
    const ids = (hits.length ? hits : base).map(t => t.id)
    return { line: `Here's what fits "${query}".`, sub: `${ids.length} · refine or filter to narrow`, ids }
  }

  async refine(kind: 'lighter' | 'shorter' | 'surprise', current: Title[]): Promise<AskResult> {
    await Promise.resolve()
    if (kind === 'lighter') {
      const ids = current.filter(t => t.genres.some(g => LIGHT.has(g))).map(t => t.id)
      return { line: 'Lighter picks.', sub: `${ids.length} · refine or filter to narrow`, ids }
    }
    if (kind === 'shorter') {
      const ids = [...current].sort((a, b) => lenMinutes(a.len) - lenMinutes(b.len)).map(t => t.id)
      return { line: 'Shortest first.', sub: `${ids.length} · refine or filter to narrow`, ids }
    }
    const pool = current.filter(t => (t.imdb ?? 0) >= 8)
    const pick = pool.length ? [pool[0].id] : current.slice(0, 1).map(t => t.id) // deterministic for tests
    return { line: 'A surprise for you.', sub: `${pick.length} · refine or filter to narrow`, ids: pick }
  }

  similar(title: Title, all: Title[]): AskResult {
    const g = new Set(title.genres)
    const ids = all
      .filter(t => t.id !== title.id && t.genres.some(x => g.has(x)))
      .map(t => ({ id: t.id, shared: t.genres.filter(x => g.has(x)).length, imdb: t.imdb ?? -Infinity }))
      .sort((a, b) => b.shared - a.shared || b.imdb - a.imdb)
      .map(x => x.id)
    return { line: `More like ${title.title}.`, sub: `${ids.length} · refine or filter to narrow`, ids }
  }
}
```
`src/services/index.ts`:
```ts
import { StubAskService } from './askService'
import type { AskService } from './askService'
// Plan 3 swaps this line for: export const askService: AskService = new ApiAskService()
export const askService: AskService = new StubAskService()
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- stub`
Expected: PASS. (Note: `surprise` and `ask` are made deterministic for tests — no `Math.random`.)

- [ ] **Step 5: Commit**

```bash
git add src/services
git commit -m "feat(frontend): AskService interface + client-side StubAskService"
```

---

### Task 12: Store ask actions + answer/thread state

**Files:**
- Modify: `frontend/src/stores/catalogue.ts`
- Test: `frontend/src/stores/__tests__/ask.test.ts`

**Interfaces:**
- Consumes: `askService` (Task 11), `AskResult`/`ThreadStep` (Task 2).
- Produces: state `answerActive, resultIds, line, sub, thread: ThreadStep[], resolving`; getter `visibleTitles` now uses the answer set as base when active; actions `submitAsk(q)`, `refine(kind)`, `moreLike(title)`, `stepThread(i)`, `clearThread()`.

- [ ] **Step 1: Write failing test**

`src/stores/__tests__/ask.test.ts`:
```ts
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCatalogueStore } from '../catalogue'
import type { Title } from '@/types'

function t(p: Partial<Title>): Title {
  return { id: 0, imdbId: null, title: '', year: 2000, services: ['plex'],
    type: 'movie', genres: [], imdb: null, len: '90 min', desc: '',
    cast: [], watched: false, rating: null, ...p }
}
const cat: Title[] = [
  t({ id: 1, title: 'Frieren', genres: ['Animation'] }),
  t({ id: 2, title: 'Coco', genres: ['Animation'] }),
  t({ id: 3, title: 'Alien', genres: ['Horror'] }),
]

beforeEach(() => setActivePinia(createPinia()))

describe('store ask actions', () => {
  it('submitAsk activates the answer, sets the base set, pushes a thread step', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    await s.submitAsk('animation')
    expect(s.answerActive).toBe(true)
    expect(s.visibleTitles.map(x => x.id).sort()).toEqual([1, 2])
    expect(s.thread.length).toBe(1)
  })

  it('filters compose within the answer set', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    await s.submitAsk('animation')
    s.setQuery('coco')
    expect(s.visibleTitles.map(x => x.id)).toEqual([2])
  })

  it('clearThread resets to the full catalogue', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    await s.submitAsk('animation'); s.clearThread()
    expect(s.answerActive).toBe(false)
    expect(s.visibleTitles).toHaveLength(3)
  })

  it('stepThread(i) restores that step and truncates later steps', async () => {
    const s = useCatalogueStore(); s.catalogue = cat
    await s.submitAsk('animation')      // step 0
    await s.refine('lighter')           // step 1
    s.stepThread(0)
    expect(s.thread.length).toBe(1)
    expect(s.resultIds.sort()).toEqual([1, 2])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- ask`
Expected: FAIL.

- [ ] **Step 3: Implement**

Add to `State`: `answerActive: boolean` (false), `resultIds: number[]` ([]), `line: string` (''), `sub: string` (''), `thread: ThreadStep[]` ([]), `resolving: boolean` (false). Import `askService` and `ThreadStep`.

Change `visibleTitles` base line to:
```ts
const base = state.answerActive
  ? state.resultIds.map(id => state.catalogue.find(t => t.id === id)).filter((t): t is Title => !!t)
  : state.catalogue.slice()
let out = base
```
(Keep the rest of the filter/sort chain. Note: when `answerActive`, `sort='trending'` should preserve the answer order — it already does since `trending` is a no-op.)

Add a private helper + actions:
```ts
applyResult(label: string, r: AskResult) {
  this.answerActive = true
  this.resultIds = r.ids
  this.line = r.line
  this.sub = r.sub
  this.thread.push({ label, line: r.line, sub: r.sub, ids: r.ids })
},
async submitAsk(q: string) {
  this.resolving = true
  try { this.applyResult(q, await askService.ask(q, this.catalogue)) }
  finally { this.resolving = false }
},
async refine(kind: 'lighter' | 'shorter' | 'surprise') {
  const current = this.resultIds.map(id => this.catalogue.find(t => t.id === id)!).filter(Boolean)
  this.resolving = true
  try { this.applyResult(`↻ ${kind}`, await askService.refine(kind, current)) }
  finally { this.resolving = false }
},
async moreLike(title: Title) {
  this.resolving = true
  try { this.applyResult(`≈ ${title.title}`, askService.similar(title, this.catalogue)) }
  finally { this.resolving = false }
},
stepThread(i: number) {
  const step = this.thread[i]; if (!step) return
  this.thread = this.thread.slice(0, i + 1)
  this.resultIds = step.ids; this.line = step.line; this.sub = step.sub; this.answerActive = true
},
clearThread() {
  this.answerActive = false; this.resultIds = []; this.line = ''; this.sub = ''; this.thread = []
},
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- ask`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/stores
git commit -m "feat(frontend): ask/refine/thread store actions composing with filters"
```

---

### Task 13: AskBar + ThreadBreadcrumb

**Files:**
- Create: `frontend/src/components/AskBar.vue`
- Create: `frontend/src/components/ThreadBreadcrumb.vue`
- Modify: `frontend/src/views/BrowseView.vue` (mount AskBar sticky at top of the browse column)
- Test: `frontend/src/components/__tests__/AskBar.test.ts`

**Interfaces:**
- Consumes: `useCatalogueStore` (`submitAsk`, `thread`, `stepThread`, `clearThread`, `resolving`).
- Produces: AskBar (✦ + input + amber Ask button) emitting submission to `store.submitAsk`; ThreadBreadcrumb rendering `Library` + a pill per thread step.

- [ ] **Step 1: Write failing test**

`src/components/__tests__/AskBar.test.ts`:
```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import AskBar from '../AskBar.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))

describe('AskBar', () => {
  it('submitting calls store.submitAsk with the input value', async () => {
    const w = mount(AskBar)
    const s = useCatalogueStore()
    const spy = vi.spyOn(s, 'submitAsk').mockResolvedValue()
    await w.get('[data-test="ask-input"]').setValue('cozy and low-stakes')
    await w.get('[data-test="ask-submit"]').trigger('click')
    expect(spy).toHaveBeenCalledWith('cozy and low-stakes')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- AskBar`
Expected: FAIL.

- [ ] **Step 3: Implement**

Port the ask-bar + thread-breadcrumb markup/CSS from the prototype (Integrated concept). AskBar local `ref` for input; submit on Enter or Ask button → `store.submitAsk(value)` then clear input. `[data-test="ask-input"]`, `[data-test="ask-submit"]`. ThreadBreadcrumb: shown `v-if="store.thread.length"`; `Library` pill `@click="store.clearThread()"`, then a pill per step (`@click="store.stepThread(i)"`), last pill amber-tinted active. Mount both at the top of the browse column in `BrowseView` (AskBar `position: sticky; top: 0`).

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- AskBar`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/AskBar.vue src/components/ThreadBreadcrumb.vue src/views/BrowseView.vue src/components/__tests__/AskBar.test.ts
git commit -m "feat(frontend): AskBar and ThreadBreadcrumb (integrated ask)"
```

---

### Task 14: AnswerContext + RefineChips + ShimmerGrid + answer-aware empty state

**Files:**
- Create: `frontend/src/components/AnswerContext.vue`
- Create: `frontend/src/components/RefineChips.vue`
- Create: `frontend/src/components/ShimmerGrid.vue`
- Modify: `frontend/src/views/BrowseView.vue`
- Test: `frontend/src/components/__tests__/AnswerContext.test.ts`

**Interfaces:**
- Consumes: `useCatalogueStore` (`answerActive`, `line`, `sub`, `resolving`, `refine`).
- Produces: AnswerContext (✦ + line + sub + RefineChips) shown when `answerActive`; ShimmerGrid shown when `resolving`; BrowseView empty-state copy switches on `answerActive`.

- [ ] **Step 1: Write failing test**

`src/components/__tests__/AnswerContext.test.ts`:
```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import AnswerContext from '../AnswerContext.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))

describe('AnswerContext', () => {
  it('renders the answer line and refine chips trigger store.refine', async () => {
    const s = useCatalogueStore()
    s.answerActive = true; s.line = 'Lighter picks.'; s.sub = '3 · refine'
    const spy = vi.spyOn(s, 'refine').mockResolvedValue()
    const w = mount(AnswerContext)
    expect(w.text()).toContain('Lighter picks.')
    await w.get('[data-test="refine-lighter"]').trigger('click')
    expect(spy).toHaveBeenCalledWith('lighter')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- AnswerContext`
Expected: FAIL.

- [ ] **Step 3: Implement**

Port AnswerContext/RefineChips/ShimmerGrid markup/CSS from the prototype. RefineChips: `Even lighter`→`refine('lighter')` (`[data-test="refine-lighter"]`), `Make it shorter`→`refine('shorter')`, `Surprise me`→`refine('surprise')`. ShimmerGrid: poster-aspect blocks pulsing via the `cuePulse` keyframe (copy keyframe from prototype). In BrowseView: show `<ShimmerGrid v-if="store.resolving" />`, else `<AnswerContext v-if="store.answerActive" />` then the grid/empty-state. Make the empty-state copy conditional:
```ts
const emptyCopy = computed(() => store.answerActive
  ? 'Nothing in this result set matches those filters — loosen a filter or clear the thread.'
  : 'Nothing in your library matches those filters.')
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- AnswerContext`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/AnswerContext.vue src/components/RefineChips.vue src/components/ShimmerGrid.vue src/views/BrowseView.vue src/components/__tests__/AnswerContext.test.ts
git commit -m "feat(frontend): answer context, refine chips, resolve shimmer"
```

---

### Task 15: Per-card find-similar wiring + keyboard shortcuts

**Files:**
- Modify: `frontend/src/views/BrowseView.vue` (wire `find-similar` → `store.moreLike`; global keydown)
- Test: `frontend/src/views/__tests__/keyboard.test.ts`

**Interfaces:**
- Consumes: `useCatalogueStore` (`moreLike`, `clearThread`).
- Produces: `find-similar` from any card calls `store.moreLike(title)`; `/` focuses the ask input, `Esc` clears the active answer/thread (or blurs).

- [ ] **Step 1: Write failing test**

`src/views/__tests__/keyboard.test.ts`:
```ts
import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import BrowseView from '../BrowseView.vue'
import { useCatalogueStore } from '@/stores/catalogue'

beforeEach(() => setActivePinia(createPinia()))

describe('Browse keyboard', () => {
  it('Esc clears an active answer', async () => {
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue([])
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: BrowseView }] })
    router.push('/'); await router.isReady()
    const w = mount(BrowseView, { attachTo: document.body, global: { plugins: [router] } })
    await flushPromises()
    const s = useCatalogueStore(); s.answerActive = true; s.thread = [{ label: 'x', line: '', sub: '', ids: [] }]
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(s.answerActive).toBe(false)
    w.unmount()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- keyboard`
Expected: FAIL.

- [ ] **Step 3: Implement**

In BrowseView: wire `<PosterGrid @find-similar="store.moreLike" />`. Add a global keydown listener in `onMounted` / removed in `onUnmounted`:
```ts
function onKey(e: KeyboardEvent) {
  const typing = e.target instanceof HTMLElement && ['INPUT', 'TEXTAREA', 'SELECT'].includes(e.target.tagName)
  if (e.key === '/' && !typing) { e.preventDefault(); document.querySelector<HTMLInputElement>('[data-test="ask-input"]')?.focus() }
  else if (e.key === 'Escape') { if (store.answerActive) store.clearThread(); else (document.activeElement as HTMLElement | null)?.blur() }
}
onMounted(() => document.addEventListener('keydown', onKey))
onUnmounted(() => document.removeEventListener('keydown', onKey))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- keyboard`
Expected: PASS.

- [ ] **Step 5: Full suite + manual pass**

```bash
npm run test
# Manual (backend running): cd frontend && npm run dev — verify ask reshapes the grid,
# thread pills step back, refine chips work, ✦ on a card asks "More like …", / and Esc work.
```

- [ ] **Step 6: Commit**

```bash
git add src/views/BrowseView.vue src/views/__tests__/keyboard.test.ts
git commit -m "feat(frontend): per-card find-similar and keyboard shortcuts"
```

---

### Task 16: Production Docker integration

**Files:**
- Modify: `Dockerfile` (enable frontend stage + copy dist)
- Modify: `.github/workflows/docker-smoke.yml` (bump `actions/checkout@v4` → `@v5`)

**Interfaces:**
- Produces: the production image builds the SPA and serves it; the existing `docker-smoke` CI exercises the full image.

- [ ] **Step 1: Enable the Dockerfile frontend stage**

Uncomment the `frontend` build stage (lines ~3–9) and the `COPY --from=frontend /app/frontend/dist ./frontend/dist` line (~27). Confirm the stage copies `frontend/package*.json`, runs `npm ci`, copies `frontend/`, runs `npm run build` (outputs `/app/frontend/dist`).

- [ ] **Step 2: Bump the deprecated action**

In `.github/workflows/docker-smoke.yml`, change `uses: actions/checkout@v4` to `uses: actions/checkout@v5`.

- [ ] **Step 3: Verify the frontend build is reproducible**

```bash
cd frontend && npm ci && npm run build   # emits dist/ ; commit Cargo.lock-equivalent: package-lock.json
```
(This machine can't run Docker — the build-and-serve verification happens in CI on push, which is the source of truth for the image.)

- [ ] **Step 4: Commit + push (triggers the smoke CI)**

```bash
git add Dockerfile .github/workflows/docker-smoke.yml frontend/package-lock.json
git commit -m "build(frontend): serve SPA from the production image; bump checkout action"
git push
```

- [ ] **Step 5: Confirm CI is green**

```bash
gh run list --limit 1
gh run watch <run-id> --exit-status
```
Expected: `docker-smoke` passes — the image now bundles and serves the real frontend.

---

## Self-Review

**Spec coverage** (each spec section → task):
- §1/§2 scope, Integrated-only, real contract → Global Constraints + Tasks 2–3.
- §3 structure → Task 1 + File Structure.
- §4 data layer (types, client, dev proxy) → Tasks 1 (proxy), 2 (types/client).
- §5 store (filters, visibleTitles, similar, user data, ask state) → Tasks 3, 4, 12.
- §6 askService seam → Task 11 (+ Task 12 wiring; Plan-3 swap point documented).
- §7 screens/components → AppHeader (6), FilterBar (7), PosterCard (5), Browse (8), Detail (9–10), AskBar/Thread (13), AnswerContext/Refine/Shimmer (14), per-card ✦ + keyboard (15).
- §8 styling (tokens, self-host fonts, no Tailwind) → Task 1 + per-component "port from prototype".
- §9 testing (Vitest store/stub/components) → tests in every task.
- §10 Docker integration → Task 16.
- §11 out-of-scope → respected (no ask engine/sync/writes-persistence/real art/auth/responsive).
- §12 open items → checkout bump folded into Task 16; poster art deferred.

**Placeholder scan:** Component tasks reference the concrete prototype file (`design_handoff_cue/cue.dc.html`) and exact token names for markup/CSS — a real source, not a TODO. All logic tasks contain complete code. No "TBD"/"add error handling"/"similar to Task N".

**Type consistency:** `Title`, `ServiceKey`, `AskResult`, `ThreadStep` defined in Task 2 and used unchanged throughout. Store getters/actions named consistently (`visibleTitles`, `similar`, `isWatched`, `ratingOf`, `submitAsk`, `refine`, `moreLike`, `stepThread`, `clearThread`, `toggleWatched`, `setRating`). `AskService` methods (`ask`/`refine`/`similar`) match between Tasks 11 and 12. `data-test` hooks consistent between components and their tests.

**Note on `refine('shorter')`:** `lenMinutes` maps `"28 eps"` → `28*24=672`, `"105 min"` → `105`, `"117 min"` → `117`, so the Task 11 test asserts the deterministic order `[2, 3, 1]` (Coco, Alien, Frieren). Series sort as long watches by design — "Make it shorter" favors short movies over multi-episode series.
