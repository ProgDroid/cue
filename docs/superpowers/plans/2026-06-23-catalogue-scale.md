# Catalogue Browse Scaling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make catalogue browse scale to cue's bounded catalogue (low tens of thousands) by trimming the list payload, adding a lazy per-title detail endpoint, and virtualizing the poster grid — without changing the load-everything-once client-side compose model.

**Architecture:** `GET /api/catalogue` returns a slim `TitleListItem` (no `desc`/`cast`, no `title_cast` join). A new `GET /api/titles/:id` returns the full `TitleDto`; `DetailView` fetches it on mount. `PosterGrid` renders only the visible row-window via a hand-rolled `useVirtualGrid` composable (JS owns column count via `--cols`; row height measured live).

**Tech Stack:** Rust + Actix-web + SQLx (runtime queries) + SQLite; Vue 3 + TypeScript + Pinia; Vitest + @vue/test-utils (jsdom).

## Global Constraints

- SQLx **runtime queries only** (`query` / `query_as` / `query_scalar`) — never the `query!` macros.
- Test DBs use `tempfile::tempdir()` + URL `format!("sqlite:{}", path.to_string_lossy().replace('\\', "/"))`; keep the `TempDir` guard bound as `_dir`.
- Clippy gate: `cargo clippy --all-targets -- -D warnings` must pass; per-item `#[allow]` with a one-line reason only — never widen the global table.
- **No new frontend runtime dependencies** (project keeps a deliberate 4-dep footprint).
- New backend modules are declared in `src/lib.rs` (none needed here — `catalogue` already exists).
- **Make git commits via the Bash tool, not PowerShell** (PowerShell prepends a UTF-8 BOM to the commit subject).
- No DB migration — `description` and `title_cast` stay in the schema; the list query simply stops reading them.

---

### Task 1: Backend — slim list DTO + `fetch_catalogue`

**Files:**
- Modify: `src/models.rs` (add `TitleListItem`, `TitleListRow`)
- Modify: `src/db/catalogue.rs:11-83` (`fetch_catalogue` returns the slim type)
- Modify: `src/routes/catalogue.rs:34-54` (test asserts no `desc`/`cast`)

**Interfaces:**
- Produces: `cue::models::TitleListItem` (Serialize); `cue::db::catalogue::fetch_catalogue(&SqlitePool) -> anyhow::Result<Vec<TitleListItem>>`.

- [ ] **Step 1: Add the slim structs to `src/models.rs`**

After the `TitleRow` struct (line 70), add:

```rust
/// Lightweight row for the catalogue list query (no `description`).
#[derive(Debug, Clone, FromRow)]
pub struct TitleListRow {
    pub id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: i64,
    #[sqlx(rename = "type")]
    pub kind: String,
    pub imdb_rating: Option<f64>,
    pub length: String,
}
```

After `TitleDto` (line 89), add:

```rust
/// Slim list shape: `TitleDto` minus the DetailView-only `desc`/`cast`.
#[derive(Debug, Clone, Serialize)]
pub struct TitleListItem {
    pub id: i64,
    #[serde(rename = "imdbId")]
    pub imdb_id: Option<String>,
    pub title: String,
    pub year: i64,
    pub services: Vec<Service>,
    #[serde(rename = "type")]
    pub kind: TitleKind,
    pub genres: Vec<String>,
    pub imdb: Option<f64>,
    pub len: String,
    pub watched: bool,
    pub rating: Option<i64>,
}
```

- [ ] **Step 2: Update the existing endpoint test to assert the slim shape**

In `src/routes/catalogue.rs`, inside `catalogue_endpoint_returns_seed`, after the existing `assert!(first["genres"].is_array());` line add:

```rust
        assert!(first.get("desc").is_none(), "list payload must not carry desc");
        assert!(first.get("cast").is_none(), "list payload must not carry cast");
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib catalogue_endpoint_returns_seed`
Expected: FAIL — `fetch_catalogue` still returns the full DTO so `desc`/`cast` are present (assertion fails).

- [ ] **Step 4: Rewrite `fetch_catalogue` to return `TitleListItem`**

Replace the whole body of `src/db/catalogue.rs` `fetch_catalogue` (lines 11-83) with:

```rust
use crate::models::{Service, TitleKind, TitleListItem, TitleListRow};

/// Fetch every title in the slim list shape (services, genres, user-data; no
/// `desc`/`cast`).
///
/// # Errors
/// Returns an error if any database query fails.
pub async fn fetch_catalogue(pool: &SqlitePool) -> anyhow::Result<Vec<TitleListItem>> {
    let rows: Vec<TitleListRow> = sqlx::query_as(
        "SELECT id, imdb_id, title, year, type, imdb_rating, length
         FROM titles ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let services =
        sqlx::query_as::<_, (i64, String)>("SELECT title_id, service FROM title_services")
            .fetch_all(pool)
            .await?;
    let mut svc_map: HashMap<i64, Vec<Service>> = HashMap::new();
    for (tid, s) in services {
        if let Some(service) = Service::parse(&s) {
            svc_map.entry(tid).or_default().push(service);
        }
    }

    let genres = sqlx::query_as::<_, (i64, String)>(
        "SELECT title_id, genre FROM title_genres ORDER BY title_id, genre",
    )
    .fetch_all(pool)
    .await?;
    let mut genre_map: HashMap<i64, Vec<String>> = HashMap::new();
    for (tid, g) in genres {
        genre_map.entry(tid).or_default().push(g);
    }

    let ratings = sqlx::query_as::<_, (String, i64)>("SELECT imdb_id, rating FROM user_ratings")
        .fetch_all(pool)
        .await?;
    let rating_map: HashMap<String, i64> = ratings.into_iter().collect();

    let watched = sqlx::query_scalar::<_, String>("SELECT DISTINCT imdb_id FROM watch_history")
        .fetch_all(pool)
        .await?;
    let watched_set: HashSet<String> = watched.into_iter().collect();

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let kind = TitleKind::parse(&r.kind).unwrap_or(TitleKind::Movie);
        let (watched, rating) = r.imdb_id.as_ref().map_or((false, None), |key| {
            (watched_set.contains(key), rating_map.get(key).copied())
        });
        out.push(TitleListItem {
            id: r.id,
            imdb_id: r.imdb_id,
            title: r.title,
            year: r.year,
            services: svc_map.remove(&r.id).unwrap_or_default(),
            kind,
            genres: genre_map.remove(&r.id).unwrap_or_default(),
            imdb: r.imdb_rating,
            len: r.length,
            watched,
            rating,
        });
    }
    Ok(out)
}
```

Remove the now-unused old imports at the top of the file (`TitleDto`, `TitleRow`) — keep `std::collections::{HashMap, HashSet}` and `sqlx::SqlitePool`. The module-level `use crate::models::...` line at the original top of the file should be deleted in favour of the one inside this snippet's preamble (or merge them; only one `use crate::models::...` should remain, importing `Service, TitleKind, TitleListItem, TitleListRow`).

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib catalogue_endpoint_returns_seed`
Expected: PASS.

- [ ] **Step 6: Clippy + full lib test**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.
Run: `cargo test --lib`
Expected: all pass.

- [ ] **Step 7: Commit (via Bash tool)**

```bash
git add src/models.rs src/db/catalogue.rs src/routes/catalogue.rs
git commit -m "feat(catalogue): slim list DTO (drop desc/cast from GET /api/catalogue)"
```

---

### Task 2: Backend — `fetch_title` + `GET /api/titles/:id`

**Files:**
- Modify: `src/db/catalogue.rs` (add `fetch_title`)
- Modify: `src/routes/catalogue.rs` (add `get_title` handler + tests)
- Modify: `src/routes/mod.rs:24` (register the route)

**Interfaces:**
- Consumes: `cue::models::{TitleDto, TitleRow}`; `fetch_catalogue` (Task 1).
- Produces: `cue::db::catalogue::fetch_title(&SqlitePool, i64) -> anyhow::Result<Option<TitleDto>>`; `cue::routes::catalogue::get_title`.

- [ ] **Step 1: Write the failing endpoint tests**

In `src/routes/catalogue.rs`, inside `mod tests`, add two tests (reusing the existing `seeded_pool` helper):

```rust
    #[actix_web::test]
    async fn title_detail_returns_full_record() {
        let (pool, _dir) = seeded_pool().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(routes::configure),
        )
        .await;

        // id 1 exists in the seed
        let req = test::TestRequest::get().uri("/api/titles/1").to_request();
        let body: Value = test::call_and_read_body_json(&app, req).await;

        assert_eq!(body["id"], 1);
        assert!(body["desc"].is_string(), "detail must carry desc");
        assert!(body["cast"].is_array(), "detail must carry cast");
        assert!(body["services"].is_array());
    }

    #[actix_web::test]
    async fn title_detail_unknown_id_is_404() {
        let (pool, _dir) = seeded_pool().await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .configure(routes::configure),
        )
        .await;

        let req = test::TestRequest::get().uri("/api/titles/999999").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib title_detail`
Expected: FAIL — route `/api/titles/{id}` (GET) not registered, `get_title` undefined.

- [ ] **Step 3: Add `fetch_title` to `src/db/catalogue.rs`**

Append to `src/db/catalogue.rs` (and add `TitleDto, TitleRow` to the `use crate::models::...` import line):

```rust
/// Fetch one fully-hydrated title (services, genres, cast, user-data).
///
/// # Errors
/// Returns an error if any database query fails.
pub async fn fetch_title(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<TitleDto>> {
    let Some(r) = sqlx::query_as::<_, TitleRow>(
        "SELECT id, imdb_id, title, year, type, imdb_rating, length, description
         FROM titles WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let services: Vec<Service> =
        sqlx::query_scalar::<_, String>("SELECT service FROM title_services WHERE title_id = ?")
            .bind(id)
            .fetch_all(pool)
            .await?
            .iter()
            .filter_map(|s| Service::parse(s))
            .collect();

    let genres = sqlx::query_scalar::<_, String>(
        "SELECT genre FROM title_genres WHERE title_id = ? ORDER BY genre",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let cast = sqlx::query_scalar::<_, String>(
        "SELECT person FROM title_cast WHERE title_id = ? ORDER BY ord",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let (watched, rating) = if let Some(key) = r.imdb_id.as_ref() {
        let rating =
            sqlx::query_scalar::<_, i64>("SELECT rating FROM user_ratings WHERE imdb_id = ?")
                .bind(key)
                .fetch_optional(pool)
                .await?;
        let watched =
            sqlx::query_scalar::<_, i64>("SELECT 1 FROM watch_history WHERE imdb_id = ? LIMIT 1")
                .bind(key)
                .fetch_optional(pool)
                .await?
                .is_some();
        (watched, rating)
    } else {
        (false, None)
    };

    let kind = TitleKind::parse(&r.kind).unwrap_or(TitleKind::Movie);
    Ok(Some(TitleDto {
        id: r.id,
        imdb_id: r.imdb_id,
        title: r.title,
        year: r.year,
        services,
        kind,
        genres,
        imdb: r.imdb_rating,
        len: r.length,
        desc: r.description,
        cast,
        watched,
        rating,
    }))
}
```

- [ ] **Step 4: Add the `get_title` handler to `src/routes/catalogue.rs`**

Change the top import to `use crate::db::catalogue::{fetch_catalogue, fetch_title};` and add, after `get_catalogue`:

```rust
pub async fn get_title(
    pool: web::Data<SqlitePool>,
    path: web::Path<i64>,
) -> impl Responder {
    let id = path.into_inner();
    match fetch_title(pool.get_ref(), id).await {
        Ok(Some(dto)) => HttpResponse::Ok().json(dto),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => {
            tracing::error!("title detail fetch failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}
```

- [ ] **Step 5: Register the route in `src/routes/mod.rs`**

After the `/catalogue` route line (line 18), add:

```rust
            .route("/titles/{id}", web::get().to(catalogue::get_title))
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib title_detail`
Expected: PASS (both).

- [ ] **Step 7: Clippy + full lib test, then commit**

Run: `cargo clippy --all-targets -- -D warnings` → clean.
Run: `cargo test --lib` → all pass.

```bash
git add src/db/catalogue.rs src/routes/catalogue.rs src/routes/mod.rs
git commit -m "feat(catalogue): add GET /api/titles/:id detail endpoint"
```

---

### Task 3: Frontend — type split + client `getTitle`

**Files:**
- Modify: `frontend/src/types.ts:4-18`
- Modify: `frontend/src/api/client.ts`
- Test: `frontend/src/api/__tests__/client.test.ts` (create if absent; else add cases)
- Modify (fixtures): `frontend/src/stores/__tests__/catalogue.test.ts`, `frontend/src/stores/__tests__/similar.test.ts`, `frontend/src/stores/__tests__/ask.test.ts`

**Interfaces:**
- Produces: `TitleListItem`, `TitleDetail`, `Title` (alias of `TitleListItem`) in `@/types`; `getTitle(id: number): Promise<TitleDetail>` and `NotFoundError` in `@/api/client`.

- [ ] **Step 1: Split the title types in `frontend/src/types.ts`**

Replace the `Title` interface (lines 4-18) with:

```ts
export interface TitleListItem {
  id: number
  imdbId: string | null
  title: string
  year: number
  services: ServiceKey[]
  type: TitleKind
  genres: string[]
  imdb: number | null
  len: string
  watched: boolean
  rating: number | null
}

/** The dominant in-store shape is the slim list item. */
export type Title = TitleListItem

export interface TitleDetail extends TitleListItem {
  desc: string
  cast: string[]
}
```

- [ ] **Step 2: Write the failing client tests**

Create `frontend/src/api/__tests__/client.test.ts`:

```ts
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { getCatalogue, getTitle, NotFoundError } from '@/api/client'
import type { TitleDetail } from '@/types'

const listItem = {
  id: 1, imdbId: 'tt1', title: 'Coco', year: 2017, services: ['disney'],
  type: 'movie', genres: ['Animation'], imdb: 8.4, len: '105 min',
  watched: false, rating: null,
}
const detail: TitleDetail = { ...listItem, type: 'movie', desc: 'A boy.', cast: ['A. Gonzalez'] }

function mockFetch(status: number, body: unknown) {
  return vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
  } as Response)
}

beforeEach(() => { vi.restoreAllMocks() })
afterEach(() => { vi.unstubAllGlobals() })

describe('api client', () => {
  it('getCatalogue accepts slim list items (no desc/cast)', async () => {
    vi.stubGlobal('fetch', mockFetch(200, [listItem]))
    const out = await getCatalogue()
    expect(out).toHaveLength(1)
    expect(out[0].id).toBe(1)
  })

  it('getTitle returns the full detail', async () => {
    vi.stubGlobal('fetch', mockFetch(200, detail))
    const out = await getTitle(1)
    expect(out.desc).toBe('A boy.')
    expect(out.cast).toEqual(['A. Gonzalez'])
  })

  it('getTitle throws NotFoundError on 404', async () => {
    vi.stubGlobal('fetch', mockFetch(404, null))
    await expect(getTitle(9)).rejects.toBeInstanceOf(NotFoundError)
  })

  it('getTitle throws on malformed detail', async () => {
    vi.stubGlobal('fetch', mockFetch(200, { id: 1 }))
    await expect(getTitle(1)).rejects.toThrow(/invalid/)
  })
})
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd frontend && npx vitest run src/api/__tests__/client.test.ts`
Expected: FAIL — `getTitle`/`NotFoundError` not exported.

- [ ] **Step 4: Update `frontend/src/api/client.ts`**

Replace the file contents with:

```ts
import type { Title, TitleDetail } from '@/types'

function isTitle(v: unknown): v is Title {
  if (typeof v !== 'object' || v === null) return false
  const r = v as Record<string, unknown>
  return typeof r.id === 'number'
    && typeof r.title === 'string'
    && typeof r.year === 'number'
    && (r.type === 'movie' || r.type === 'series')
    && Array.isArray(r.services)
    && Array.isArray(r.genres)
    && typeof r.len === 'string'
    && typeof r.watched === 'boolean'
}

function isTitleDetail(v: unknown): v is TitleDetail {
  if (!isTitle(v)) return false
  const r = v as Record<string, unknown>
  return typeof r.desc === 'string' && Array.isArray(r.cast)
}

export class NotFoundError extends Error {}

export async function getCatalogue(): Promise<Title[]> {
  const res = await fetch('/api/catalogue')
  if (!res.ok) {
    throw new Error(`Failed to load catalogue (HTTP ${res.status})`)
  }
  const data: unknown = await res.json()
  if (!Array.isArray(data)) {
    throw new Error('catalogue response is not an array')
  }
  if (!data.every(isTitle)) {
    throw new Error('catalogue response contains an invalid title')
  }
  return data
}

export async function getTitle(id: number): Promise<TitleDetail> {
  const res = await fetch(`/api/titles/${id}`)
  if (res.status === 404) {
    throw new NotFoundError(`Title ${id} not found`)
  }
  if (!res.ok) {
    throw new Error(`Failed to load title (HTTP ${res.status})`)
  }
  const data: unknown = await res.json()
  if (!isTitleDetail(data)) {
    throw new Error('title detail response is invalid')
  }
  return data
}
```

- [ ] **Step 5: Run the client tests to verify they pass**

Run: `cd frontend && npx vitest run src/api/__tests__/client.test.ts`
Expected: PASS.

- [ ] **Step 6: Strip `desc`/`cast` from `Title` fixtures in existing tests**

Find every Title fixture that still sets `desc`/`cast`:

Run: `cd frontend && npx grep -rn "desc:\|cast:" src/stores/__tests__ src/services/__tests__ 2>/dev/null || rg -n "desc:|cast:" src/stores/__tests__ src/services/__tests__`

In each match that builds a `Title`/`Title[]` fixture (e.g. the `t(p: Partial<Title>)` helper in `catalogue.test.ts`, and any fixtures in `similar.test.ts` / `ask.test.ts`), delete the `desc: ...` and `cast: ...` properties. (They are `Title` values now, which no longer have those fields.) Do **not** touch `DetailView.test.ts` / `DetailWatched.test.ts` — Task 4 owns those.

- [ ] **Step 7: Typecheck + full frontend test run**

Run: `cd frontend && npm run build`
Expected: vue-tsc passes (no leftover `desc`/`cast` on `Title`).
Run: `cd frontend && npm test`
Expected: all pass (DetailView tests may still pass here because the store path renders; Task 4 hardens them).

- [ ] **Step 8: Commit**

```bash
git add frontend/src/types.ts frontend/src/api/client.ts frontend/src/api/__tests__/client.test.ts frontend/src/stores/__tests__ frontend/src/services/__tests__
git commit -m "feat(frontend): split Title/TitleDetail types, add getTitle client fn"
```

---

### Task 4: Frontend — `DetailView` fetches its own record

**Files:**
- Modify: `frontend/src/views/DetailView.vue:1-47` (script) + template root (lines 49-50, 60, 82, 119, 124, 132, 138, 144)
- Modify: `frontend/src/views/__tests__/DetailView.test.ts`
- Modify: `frontend/src/views/__tests__/DetailWatched.test.ts`

**Interfaces:**
- Consumes: `getTitle`, `NotFoundError` (Task 3); `TitleDetail` (Task 3); `store.similar` (unchanged).

- [ ] **Step 1: Update `DetailView.test.ts` to drive the detail via `getTitle`**

Replace `DetailView.test.ts` with:

```ts
import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import DetailView from '../DetailView.vue'
import { useCatalogueStore } from '@/stores/catalogue'
import * as client from '@/api/client'
import type { Title, TitleDetail } from '@/types'

const listItem: Title = {
  id: 5, imdbId: 'tt5', title: 'Coco', year: 2017, services: ['disney'],
  type: 'movie', genres: ['Animation', 'Musical'], imdb: 8.4, len: '105 min',
  watched: false, rating: null,
}
const detail: TitleDetail = { ...listItem, desc: 'A boy and music.', cast: ['Anthony Gonzalez'] }

beforeEach(() => { setActivePinia(createPinia()); vi.restoreAllMocks() })

async function mountAt(id: number) {
  const router = createRouter({ history: createMemoryHistory(), routes: [
    { path: '/', component: { template: '<div>home</div>' } },
    { path: '/title/:id', component: DetailView },
  ] })
  const s = useCatalogueStore(); s.catalogue = [listItem]
  router.push(`/title/${id}`); await router.isReady()
  const w = mount(DetailView, { global: { plugins: [router] } })
  await flushPromises()
  return w
}

describe('DetailView', () => {
  it('renders the fetched title facts and cast', async () => {
    vi.spyOn(client, 'getTitle').mockResolvedValue(detail)
    const w = await mountAt(5)
    expect(w.text()).toContain('Coco')
    expect(w.text()).toContain('Anthony Gonzalez')
    expect(w.text()).toContain('2017')
    expect(w.find('.poster img').attributes('src')).toContain('/poster')
    expect(w.find('.backdrop img').attributes('src')).toContain('/backdrop')
  })

  it('shows a not-found panel on 404', async () => {
    vi.spyOn(client, 'getTitle').mockRejectedValue(new client.NotFoundError('nope'))
    const w = await mountAt(999)
    expect(w.text()).toContain('Title not found')
  })
})
```

- [ ] **Step 2: Update `DetailWatched.test.ts` to drive the detail via `getTitle`**

In `DetailWatched.test.ts`: (a) change `makeTitle` to `makeDetail` returning `TitleDetail` (it already sets `desc`/`cast`, so keep them and change the return type + import `TitleDetail`); (b) in `mountDetail`, before `mount`, add `vi.spyOn(await import('@/api/client'), 'getTitle').mockResolvedValue(title)` and keep `store.catalogue = [title]` (a `TitleDetail` is assignable to `Title`); (c) add `vi` is already imported. Concretely, replace the top of the file's fixture + `mountDetail`:

```ts
import * as client from '@/api/client'
import type { TitleDetail } from '@/types'

function makeDetail(over: Partial<TitleDetail> = {}): TitleDetail {
  return {
    id: 5, imdbId: 'tt0000005', title: 'Coco', year: 2017, services: ['disney'],
    type: 'movie', genres: ['Animation'], imdb: 8.4, len: '105 min',
    desc: 'A boy and music.', cast: ['Anthony Gonzalez'], watched: false, rating: null,
    ...over,
  }
}

async function mountDetail(title: TitleDetail) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: { template: '<div>home</div>' } },
      { path: '/title/:id', component: DetailView },
    ],
  })
  vi.spyOn(client, 'getTitle').mockResolvedValue(title)
  const store = useCatalogueStore()
  store.catalogue = [title]
  router.push(`/title/${title.id}`)
  await router.isReady()
  const wrapper = mount(DetailView, { global: { plugins: [router] } })
  await flushPromises()
  return { wrapper, store }
}
```

Then update the three `mountDetail(makeTitle(...))` call sites to `mountDetail(makeDetail(...))`.

- [ ] **Step 3: Run the DetailView tests to verify they fail**

Run: `cd frontend && npx vitest run src/views/__tests__/DetailView.test.ts src/views/__tests__/DetailWatched.test.ts`
Expected: FAIL — DetailView still reads `store.catalogue.find` (no `desc`/`cast` there now → cast/desc empty; not-found panel absent).

- [ ] **Step 4: Rewrite the `DetailView.vue` script block**

Replace lines 1-47 (the `<script setup>` block) with:

```ts
<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useCatalogueStore } from '@/stores/catalogue'
import { getTitle, NotFoundError } from '@/api/client'
import type { TitleDetail } from '@/types'
import ServicePill from '@/components/ServicePill.vue'
import StarRating from '@/components/StarRating.vue'
import { posterPlaceholder, monogram } from '@/design/tokens'

const route = useRoute()
const router = useRouter()
const store = useCatalogueStore()

const id = computed(() => Number(route.params.id))
const detail = ref<TitleDetail | null>(null)
const loading = ref(true)
const notFound = ref(false)

async function loadDetail(tid: number) {
  loading.value = true
  notFound.value = false
  detail.value = null
  try {
    detail.value = await getTitle(tid)
  } catch (e) {
    notFound.value = e instanceof NotFoundError ? true : true
  } finally {
    loading.value = false
  }
}

const ph = computed(() => detail.value ? posterPlaceholder(detail.value.title) : null)
const mono = computed(() => detail.value ? monogram(detail.value.title) : '')

const factLine = computed(() => {
  if (!detail.value) return ''
  const kind = detail.value.type === 'movie' ? 'Movie' : 'Series'
  return `${detail.value.year} · ${kind} · ${detail.value.len} · ${detail.value.genres.join(', ')}`
})

const similar = computed(() => detail.value ? store.similar(id.value) : [])

function back() { router.push('/') }

const idIsWatched = computed(() => store.isWatched(id.value))
const idRating = computed(() => store.ratingOf(id.value))
const canRate = computed(() => !!detail.value?.imdbId)
function toggleWatched() { store.toggleWatched(id.value) }
function setRating(n: number) { store.setRating(id.value, n) }
function clearRating() { store.clearRating(id.value) }

onMounted(() => {
  loadDetail(id.value)
  if (store.catalogue.length === 0) store.load()
})
watch(id, (n) => loadDetail(n))

const posterFailed = ref(false)
const backdropFailed = ref(false)
const simFailed = ref<Record<number, boolean>>({})
</script>
```

(The `notFound.value = ... ? true : true` is intentional: any fetch failure shows the not-found panel; kept as a single branch so a future "error vs missing" split has an obvious seam.)

- [ ] **Step 5: Update the `DetailView.vue` template to use `detail` + add loading/missing states**

In the template: change the root `<div v-if="title && ph" class="detail-root">` to `<div v-if="detail && ph" class="detail-root">`, and replace every `title.` reference inside with `detail.` (occurrences: backdrop `:src` line ~60, poster `:src` line ~82, `title.services` line ~119, `title.imdb` lines ~124/126, `title.title` line ~132, `title.desc` line ~138, `title.cast` line ~144). Immediately after the closing `</div>` of `.detail-root`, before the end of the template, add:

```html
  <div v-else-if="loading" class="detail-state">Loading…</div>
  <div v-else class="detail-state">
    <p>Title not found.</p>
    <button class="back-btn" @click="back">← Library</button>
  </div>
```

Add a style rule to the `<style scoped>` block:

```css
.detail-state {
  padding: 80px 22px;
  text-align: center;
  color: var(--text-faint, #5f6570);
  font-family: var(--font-ui, 'Hanken Grotesk', system-ui, sans-serif);
}
```

- [ ] **Step 6: Run the DetailView tests to verify they pass**

Run: `cd frontend && npx vitest run src/views/__tests__/DetailView.test.ts src/views/__tests__/DetailWatched.test.ts`
Expected: PASS.

- [ ] **Step 7: Full frontend typecheck + test, then commit**

Run: `cd frontend && npm run build` → passes.
Run: `cd frontend && npm test` → all pass.

```bash
git add frontend/src/views/DetailView.vue frontend/src/views/__tests__/DetailView.test.ts frontend/src/views/__tests__/DetailWatched.test.ts
git commit -m "feat(frontend): DetailView fetches full record via GET /api/titles/:id"
```

---

### Task 5: Frontend — grid constants + `computeWindow` + `useVirtualGrid`

**Files:**
- Create: `frontend/src/design/grid.ts`
- Create: `frontend/src/composables/computeWindow.ts`
- Create: `frontend/src/composables/useVirtualGrid.ts`
- Test: `frontend/src/composables/__tests__/computeWindow.test.ts`

**Interfaces:**
- Produces: constants `MIN_COL`, `COL_GAP`, `ROW_GAP`; `computeWindow(m: GridMetrics): GridWindow`; `useVirtualGrid(opts)` returning `{ cols, startIndex, endIndex, topSpacer, bottomSpacer }` (all `ComputedRef<number>`).

- [ ] **Step 1: Create the shared grid constants**

`frontend/src/design/grid.ts`:

```ts
// Mirror of the `.poster-grid` CSS. JS owns the column count for virtualization,
// so these MUST match the stylesheet values.
export const MIN_COL = 158 // px — minmax() floor
export const COL_GAP = 18  // px — column gap
export const ROW_GAP = 22  // px — row gap
```

- [ ] **Step 2: Write the failing `computeWindow` tests**

`frontend/src/composables/__tests__/computeWindow.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { computeWindow } from '@/composables/computeWindow'

const base = { containerWidth: 800, rowHeight: 172, scrollOffset: 0, viewportH: 400, itemCount: 100, overscanRows: 3 }

describe('computeWindow', () => {
  it('derives column count from width via the auto-fill formula', () => {
    // floor((800 + 18) / (158 + 18)) = floor(818/176) = 4
    expect(computeWindow(base).cols).toBe(4)
  })

  it('renders everything (spacers 0) before rowHeight is measured', () => {
    const w = computeWindow({ ...base, rowHeight: 0 })
    expect(w.startIndex).toBe(0)
    expect(w.endIndex).toBe(100)
    expect(w.topSpacer).toBe(0)
    expect(w.bottomSpacer).toBe(0)
  })

  it('windows to the visible rows + overscan at the top', () => {
    const w = computeWindow(base)
    // startRow = max(0, floor(0/172) - 3) = 0
    // endRow = min(25, ceil(400/172) + 3) = min(25, 3 + 3) = 6 ; cols 4 -> 24
    expect(w.startIndex).toBe(0)
    expect(w.endIndex).toBe(24)
    expect(w.topSpacer).toBe(0)
    expect(w.bottomSpacer).toBe((25 - 6) * 172)
  })

  it('windows in the middle with overscan both sides', () => {
    const w = computeWindow({ ...base, scrollOffset: 172 * 10 })
    // startRow = floor(1720/172) - 3 = 10 - 3 = 7 -> startIndex 28
    // endRow = ceil((1720+400)/172) + 3 = ceil(12.3) + 3 = 13 + 3 = 16 -> endIndex 64
    expect(w.startIndex).toBe(28)
    expect(w.endIndex).toBe(64)
    expect(w.topSpacer).toBe(7 * 172)
  })

  it('clamps at the bottom (no negative bottom spacer)', () => {
    const w = computeWindow({ ...base, scrollOffset: 172 * 1000 })
    expect(w.endIndex).toBe(100)
    expect(w.bottomSpacer).toBe(0)
  })

  it('falls back to one column at zero width', () => {
    expect(computeWindow({ ...base, containerWidth: 0 }).cols).toBe(1)
  })
})
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd frontend && npx vitest run src/composables/__tests__/computeWindow.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement `computeWindow`**

`frontend/src/composables/computeWindow.ts`:

```ts
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd frontend && npx vitest run src/composables/__tests__/computeWindow.test.ts`
Expected: PASS.

- [ ] **Step 6: Implement the `useVirtualGrid` composable**

`frontend/src/composables/useVirtualGrid.ts`:

```ts
import { ref, computed, onMounted, onUnmounted, watch, type Ref, type ComputedRef } from 'vue'
import { computeWindow } from './computeWindow'

// Persists across remounts (back-nav from DetailView) so spacer height is
// correct on the first frame and the browser can restore scroll position.
let cachedRowHeight = 0

export interface UseVirtualGridOptions {
  containerEl: Ref<HTMLElement | null>
  rowHeight: Ref<number>
  itemCount: Ref<number>
  overscanRows?: number
}

export interface UseVirtualGrid {
  cols: ComputedRef<number>
  startIndex: ComputedRef<number>
  endIndex: ComputedRef<number>
  topSpacer: ComputedRef<number>
  bottomSpacer: ComputedRef<number>
}

export function useVirtualGrid(opts: UseVirtualGridOptions): UseVirtualGrid {
  const overscanRows = opts.overscanRows ?? 3
  const containerWidth = ref(0)
  const scrollY = ref(0)
  const viewportH = ref(0)
  const gridTop = ref(0)

  function readScroll() {
    scrollY.value = window.scrollY
    viewportH.value = window.innerHeight
    const el = opts.containerEl.value
    gridTop.value = el ? el.getBoundingClientRect().top + window.scrollY : 0
  }

  let ro: ResizeObserver | null = null

  onMounted(() => {
    if (opts.rowHeight.value === 0 && cachedRowHeight > 0) {
      opts.rowHeight.value = cachedRowHeight
    }
    readScroll()
    const el = opts.containerEl.value
    if (el) {
      ro = new ResizeObserver((entries) => {
        containerWidth.value = entries[0].contentRect.width
        readScroll()
      })
      ro.observe(el)
      containerWidth.value = el.clientWidth
    }
    window.addEventListener('scroll', readScroll, { passive: true })
    window.addEventListener('resize', readScroll, { passive: true })
  })

  onUnmounted(() => {
    ro?.disconnect()
    window.removeEventListener('scroll', readScroll)
    window.removeEventListener('resize', readScroll)
  })

  watch(opts.rowHeight, (h) => { if (h > 0) cachedRowHeight = h })

  const win = computed(() => computeWindow({
    containerWidth: containerWidth.value,
    rowHeight: opts.rowHeight.value,
    scrollOffset: Math.max(0, scrollY.value - gridTop.value),
    viewportH: viewportH.value,
    itemCount: opts.itemCount.value,
    overscanRows,
  }))

  return {
    cols: computed(() => win.value.cols),
    startIndex: computed(() => win.value.startIndex),
    endIndex: computed(() => win.value.endIndex),
    topSpacer: computed(() => win.value.topSpacer),
    bottomSpacer: computed(() => win.value.bottomSpacer),
  }
}
```

- [ ] **Step 7: Typecheck + commit**

Run: `cd frontend && npm run build` → passes.

```bash
git add frontend/src/design/grid.ts frontend/src/composables/computeWindow.ts frontend/src/composables/useVirtualGrid.ts frontend/src/composables/__tests__/computeWindow.test.ts
git commit -m "feat(frontend): add useVirtualGrid composable + computeWindow math"
```

---

### Task 6: Frontend — virtualize `PosterGrid`

**Files:**
- Create: `frontend/src/test/setup.ts` (ResizeObserver mock)
- Modify: `frontend/vite.config.ts:16-19` (register `setupFiles`)
- Modify: `frontend/src/components/PosterGrid.vue`
- Test: `frontend/src/components/__tests__/PosterGrid.test.ts`

**Interfaces:**
- Consumes: `useVirtualGrid` (Task 5); `ROW_GAP` (Task 5); `Title` (Task 3).

- [ ] **Step 1: Add a ResizeObserver mock test-setup file**

`frontend/src/test/setup.ts`:

```ts
// jsdom has no ResizeObserver. This mock fires the callback immediately on
// observe() with the element's current clientWidth so virtualization math runs
// deterministically in tests.
class ResizeObserverMock {
  private cb: ResizeObserverCallback
  constructor(cb: ResizeObserverCallback) { this.cb = cb }
  observe(el: Element) {
    const width = (el as HTMLElement).clientWidth || 0
    this.cb([{ contentRect: { width } } as ResizeObserverEntry], this as unknown as ResizeObserver)
  }
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver
```

- [ ] **Step 2: Register the setup file in `frontend/vite.config.ts`**

Change the `test` block (lines 16-19) to:

```ts
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
  },
```

- [ ] **Step 3: Write the failing `PosterGrid` test**

`frontend/src/components/__tests__/PosterGrid.test.ts`:

```ts
import { mount, flushPromises } from '@vue/test-utils'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import PosterGrid from '@/components/PosterGrid.vue'
import type { Title } from '@/types'

function t(id: number): Title {
  return {
    id, imdbId: null, title: `T${id}`, year: 2000, services: ['plex'],
    type: 'movie', genres: [], imdb: null, len: '90 min', watched: false, rating: null,
  }
}

beforeEach(() => setActivePinia(createPinia()))

describe('PosterGrid virtualization', () => {
  it('renders only the visible window and sets --cols', async () => {
    // 4 columns at width 800; rowHeight 172; viewport 400 -> 6 rows -> 24 cards
    vi.stubGlobal('innerHeight', 400)
    Object.defineProperty(window, 'scrollY', { value: 0, configurable: true })

    const titles = Array.from({ length: 100 }, (_, i) => t(i + 1))
    const w = mount(PosterGrid, {
      props: { titles },
      attachTo: document.body,
    })

    // Force deterministic measurements: container width + card height.
    const container = w.element as HTMLElement
    Object.defineProperty(container, 'clientWidth', { value: 800, configurable: true })
    // every element reports offsetHeight 150 so rowHeight = 150 + ROW_GAP(22) = 172
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { value: 150, configurable: true })

    // Re-trigger measurement now that sizes are defined.
    window.dispatchEvent(new Event('resize'))
    await flushPromises()

    const grid = w.find('.poster-grid')
    expect(grid.attributes('style')).toContain('--cols: 4')
    expect(w.findAllComponents({ name: 'PosterCard' }).length).toBeLessThan(100)
  })

  it('renders all cards when the set is smaller than a viewport', async () => {
    const titles = Array.from({ length: 6 }, (_, i) => t(i + 1))
    const w = mount(PosterGrid, { props: { titles }, attachTo: document.body })
    await flushPromises()
    expect(w.findAllComponents({ name: 'PosterCard' }).length).toBe(6)
  })
})
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cd frontend && npx vitest run src/components/__tests__/PosterGrid.test.ts`
Expected: FAIL — `--cols` not set / all 100 cards rendered (not yet virtualized).

- [ ] **Step 5: Rewrite `PosterGrid.vue`**

```vue
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import type { Title } from '@/types'
import { useCatalogueStore } from '@/stores/catalogue'
import { useVirtualGrid } from '@/composables/useVirtualGrid'
import { ROW_GAP } from '@/design/grid'
import PosterCard from '@/components/PosterCard.vue'

const props = defineProps<{ titles: Title[] }>()
const emit = defineEmits<{
  select: [id: number]
  'find-similar': [t: Title]
}>()

const store = useCatalogueStore()

const containerEl = ref<HTMLElement | null>(null)
const gridEl = ref<HTMLElement | null>(null)
const rowHeight = ref(0)
const itemCount = computed(() => props.titles.length)

const { cols, startIndex, endIndex, topSpacer, bottomSpacer } = useVirtualGrid({
  containerEl,
  rowHeight,
  itemCount,
})

const visible = computed(() => props.titles.slice(startIndex.value, endIndex.value))

function measureRow() {
  const card = gridEl.value?.firstElementChild as HTMLElement | null
  if (card) rowHeight.value = card.offsetHeight + ROW_GAP
}

let ro: ResizeObserver | null = null
onMounted(() => {
  if (gridEl.value) {
    ro = new ResizeObserver(() => measureRow())
    ro.observe(gridEl.value)
  }
  void nextTick(measureRow)
})
onUnmounted(() => ro?.disconnect())
// re-measure when the rendered set changes (e.g. filter narrows the grid)
watch(visible, () => void nextTick(measureRow))
</script>

<template>
  <div ref="containerEl" class="poster-grid-virtual">
    <div :style="{ height: topSpacer + 'px' }" />
    <div ref="gridEl" class="poster-grid" :style="{ '--cols': cols }">
      <PosterCard
        v-for="t in visible"
        :key="t.id"
        :title="t"
        :watched="store.isWatched(t.id)"
        @select="emit('select', $event)"
        @find-similar="emit('find-similar', $event)"
      />
    </div>
    <div :style="{ height: bottomSpacer + 'px' }" />
  </div>
</template>

<style scoped>
.poster-grid {
  display: grid;
  grid-template-columns: repeat(var(--cols, 1), minmax(0, 1fr));
  gap: 22px 18px;
  padding: 6px 22px 40px;
}
</style>
```

- [ ] **Step 6: Run the PosterGrid test to verify it passes**

Run: `cd frontend && npx vitest run src/components/__tests__/PosterGrid.test.ts`
Expected: PASS.

- [ ] **Step 7: Full frontend typecheck + test suite**

Run: `cd frontend && npm run build` → passes.
Run: `cd frontend && npm test` → all pass (confirm `keyboard.test.ts` and the existing BrowseView/grid tests are green).

- [ ] **Step 8: Commit**

```bash
git add frontend/src/test/setup.ts frontend/vite.config.ts frontend/src/components/PosterGrid.vue frontend/src/components/__tests__/PosterGrid.test.ts
git commit -m "feat(frontend): virtualize PosterGrid rendering (window-scroll)"
```

---

### Task 7: Wrap-up — backlog + full verification

**Files:**
- Modify: `docs/superpowers/deferred-followups.md`

- [ ] **Step 1: Mark the backlog item done**

In `docs/superpowers/deferred-followups.md`, under "Scale — now live-relevant", update the first bullet (`GET /api/catalogue` returns the entire library…) to note: slim list DTO + lazy `GET /api/titles/:id` detail + window-scroll `PosterGrid` virtualization shipped (spec `2026-06-23-cue-catalogue-scale-design.md`, plan `2026-06-23-catalogue-scale.md`); server-side pagination intentionally **not** done (D1 bounded catalogue). Add a live-verify note: on a real ~5k catalogue confirm the payload is materially smaller, DetailView still shows desc/cast, and the grid renders a constant window while scrolling.

- [ ] **Step 2: Full backend gate**

Run: `cargo clippy --all-targets -- -D warnings` → clean.
Run: `cargo test` → all pass.

- [ ] **Step 3: Full frontend gate**

Run: `cd frontend && npm run build` → passes.
Run: `cd frontend && npm test` → all pass.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/deferred-followups.md
git commit -m "docs: mark catalogue-scale follow-up done (slim list + detail + virtualization)"
```

---

## Self-Review

**Spec coverage:**
- Part 1 (slim list DTO + detail endpoint) → Tasks 1, 2. ✓
- Part 2 (frontend types + client) → Task 3. ✓
- Part 3 (DetailView fetches detail; loading/404; similar from store) → Task 4. ✓
- Part 4 (constants, computeWindow, useVirtualGrid, --cols authority, measured rowHeight, spacers, module-cache, jsdom testability) → Tasks 5, 6. ✓
- Risks: CSS/JS desync (Task 6 `--cols`), variable card height (Task 6 measureRow), scroll restoration (Task 5 cachedRowHeight), keyboard nav (Task 6 step 7 verifies `keyboard.test.ts`), jsdom limits (Task 6 setup mock). ✓
- No migration (Global Constraints). ✓

**Placeholder scan:** No TBD/TODO; every code/test step shows full content. ✓

**Type consistency:** `TitleListItem`/`TitleDetail`/`Title` consistent across Tasks 3–6; `fetch_catalogue -> Vec<TitleListItem>` (Task 1) and `fetch_title -> Option<TitleDto>` (Task 2) match handler usage; `computeWindow`/`GridMetrics`/`GridWindow` consistent between Tasks 5 and 6; `useVirtualGrid` return shape matches PosterGrid destructure. ✓
