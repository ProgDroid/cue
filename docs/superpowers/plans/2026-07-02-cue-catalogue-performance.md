# Catalogue Page Performance & Loading UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the ~5k-title Browse page from freezing/crashing and give it a proper initial-load indicator, without adding server-side pagination.

**Architecture:** Four independent frontend-only changes: (1) the virtualizer estimates row height instead of rendering the whole catalogue before it is measured; (2) the catalogue is stored non-reactively via `markRaw`; (3) a delayed-flag composable drives a `ShimmerGrid` during the initial fetch with an anti-flicker threshold; (4) the header search input is debounced. No backend, no migration, no API change.

**Tech Stack:** Vue 3 (`<script setup>`), Pinia, TypeScript, Vitest + `@vue/test-utils` (jsdom).

## Global Constraints

- **Frontend only.** No files under `src/`, no migration, no API change.
- **Spec:** `docs/superpowers/specs/2026-07-02-cue-catalogue-performance-design.md`.
- **The catalogue store stays synchronous and pure.** Existing store tests call `setQuery`/`setService` then read `visibleTitles` synchronously — do not move debouncing into the store; it lives at the `AppHeader` input edge.
- **JS owns `--cols`.** `MIN_COL`/`COL_GAP`/`ROW_GAP` in `frontend/src/design/grid.ts` mirror the `.poster-grid` CSS and must keep matching it.
- **Test commands** (run from `frontend/`): `npm test` (vitest run), `npm run build` (`vue-tsc -b && vite build` — type-check + build).
- **Commit style:** conventional commits. Commit after each task's tests pass.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `frontend/src/design/grid.ts` | Grid geometry constants shared by CSS-mirroring JS | Add `META_BLOCK` |
| `frontend/src/composables/computeWindow.ts` | Pure window math for virtualization | Estimate-based fallback; never render-all |
| `frontend/src/composables/__tests__/computeWindow.test.ts` | Tests for the above | Replace the render-all test; add estimate tests |
| `frontend/src/stores/catalogue.ts` | Catalogue + view state | `markRaw` the catalogue in `load()` |
| `frontend/src/stores/__tests__/catalogue.test.ts` | Store tests | Add non-reactive + seeding test |
| `frontend/src/composables/useDelayedFlag.ts` (new) | Boolean ref that turns true only after a source stays truthy for N ms | Create |
| `frontend/src/composables/__tests__/useDelayedFlag.test.ts` (new) | Tests for the above | Create |
| `frontend/src/views/BrowseView.vue` | Browse page layout | Loading indicator + gate empty-state; add error branch |
| `frontend/src/views/__tests__/BrowseView.test.ts` | Browse view tests | Add loading/empty/error cases |
| `frontend/src/composables/debounce.ts` (new) | Pure debounce utility | Create |
| `frontend/src/composables/__tests__/debounce.test.ts` (new) | Tests for the above | Create |
| `frontend/src/components/AppHeader.vue` | Header + search input | Local input ref + debounced `setQuery` |
| `frontend/src/components/__tests__/AppHeader.test.ts` | Header tests | Add debounce case |

---

## Task 1: Bounded virtualization window (estimate row height)

Root cause #1: `computeWindow` returns `endIndex: itemCount` when `rowHeight <= 0`, so the cold first paint mounts all ~5k cards. Replace that with an estimate so the first frame is already windowed.

**Files:**
- Modify: `frontend/src/design/grid.ts`
- Modify: `frontend/src/composables/computeWindow.ts`
- Test: `frontend/src/composables/__tests__/computeWindow.test.ts`

**Interfaces:**
- Consumes: `MIN_COL`, `COL_GAP`, `ROW_GAP` from `@/design/grid`.
- Produces:
  - `META_BLOCK: number` in `@/design/grid`.
  - `estimateRowHeight(containerWidth: number, cols: number): number` in `computeWindow.ts`.
  - `computeWindow(m: GridMetrics): GridWindow` — unchanged signature; behavior on `rowHeight <= 0` now returns a bounded window.

- [ ] **Step 1: Add the `META_BLOCK` constant**

In `frontend/src/design/grid.ts`, append after `ROW_GAP`:

```ts
// px — approximate height of the title + meta line rendered under each poster.
// Used only to estimate row height before the DOM is measured; the real height
// replaces it on the next tick, so a rough value is fine.
export const META_BLOCK = 56
```

- [ ] **Step 2: Write the failing tests**

Replace the existing `it('renders everything (spacers 0) before rowHeight is measured', ...)` block in `frontend/src/composables/__tests__/computeWindow.test.ts` (currently lines ~12-18) with the two tests below, and add the import:

```ts
import { computeWindow, estimateRowHeight } from '@/composables/computeWindow'
```

```ts
  it('windows to a bounded set (not the whole list) before rowHeight is measured', () => {
    // rowHeight 0 but width known -> uses estimateRowHeight, so we render a
    // small window instead of all 100 items. Regression guard for the OOM bug.
    const w = computeWindow({ ...base, rowHeight: 0 })
    expect(w.startIndex).toBe(0)
    expect(w.endIndex).toBeGreaterThan(0)
    expect(w.endIndex).toBeLessThan(base.itemCount)
    expect(w.topSpacer).toBe(0)
  })

  it('renders only a tiny first window when both width and height are unknown', () => {
    const w = computeWindow({ ...base, rowHeight: 0, containerWidth: 0 })
    expect(w.endIndex).toBeLessThan(base.itemCount)
  })

  it('estimateRowHeight returns a positive height for a known width, 0 for unknown', () => {
    expect(estimateRowHeight(800, 4)).toBeGreaterThan(0)
    expect(estimateRowHeight(0, 1)).toBe(0)
  })
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd frontend && npm test -- computeWindow`
Expected: FAIL — `estimateRowHeight` is not exported; the render-all branch returns `endIndex === 100`.

- [ ] **Step 4: Implement the estimate + bounded fallback**

Replace the full contents of `frontend/src/composables/computeWindow.ts` with:

```ts
import { MIN_COL, COL_GAP, ROW_GAP, META_BLOCK } from '@/design/grid'

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

// Estimated row height used before the DOM has been measured. The poster is
// aspect-ratio 2/3, so posterHeight = cardWidth * 1.5; META_BLOCK covers the
// title + meta line below it. Rough is fine — overscan absorbs the error and
// measureRow() replaces this with the real height on the next tick.
export function estimateRowHeight(containerWidth: number, cols: number): number {
  if (containerWidth <= 0 || cols <= 0) return 0
  const cardWidth = (containerWidth + COL_GAP) / cols - COL_GAP
  return cardWidth * 1.5 + META_BLOCK + ROW_GAP
}

export function computeWindow(m: GridMetrics): GridWindow {
  const cols = m.containerWidth > 0
    ? Math.max(1, Math.floor((m.containerWidth + COL_GAP) / (MIN_COL + COL_GAP)))
    : 1
  const totalRows = Math.ceil(m.itemCount / cols)

  // Use the measured row height when available, otherwise an estimate. NEVER
  // fall back to rendering every item — that mounts the whole catalogue at once
  // (the first-paint freeze + OOM crash this fix exists to prevent).
  const rowHeight = m.rowHeight > 0 ? m.rowHeight : estimateRowHeight(m.containerWidth, cols)

  if (rowHeight <= 0) {
    // Width is unknown too (very first synchronous render, pre-measure). Render
    // a small first window so we never mount the entire list.
    const firstWindow = Math.min(m.itemCount, cols * (m.overscanRows + 4))
    return { cols, startIndex: 0, endIndex: firstWindow, topSpacer: 0, bottomSpacer: 0, totalRows }
  }

  const startRow = Math.max(0, Math.floor(m.scrollOffset / rowHeight) - m.overscanRows)
  const endRow = Math.min(totalRows, Math.ceil((m.scrollOffset + m.viewportH) / rowHeight) + m.overscanRows)
  const startIndex = startRow * cols
  const endIndex = Math.min(m.itemCount, endRow * cols)
  const topSpacer = startRow * rowHeight
  const bottomSpacer = Math.max(0, (totalRows - endRow) * rowHeight)

  return { cols, startIndex, endIndex, topSpacer, bottomSpacer, totalRows }
}
```

- [ ] **Step 5: Run the full computeWindow + useVirtualGrid suites to verify pass**

Run: `cd frontend && npm test -- computeWindow useVirtualGrid`
Expected: PASS. (The measured-rowHeight tests are unchanged; only the unmeasured path changed.)

- [ ] **Step 6: Commit**

```bash
git add frontend/src/design/grid.ts frontend/src/composables/computeWindow.ts frontend/src/composables/__tests__/computeWindow.test.ts
git commit -m "fix(grid): estimate row height so first paint is windowed, not render-all"
```

---

## Task 2: Non-reactive catalogue store

Root cause #2: the fetched array is stored deeply reactive (~20k proxies) for no benefit — watched/ratings already live in separate reactive maps. `markRaw` it.

**Files:**
- Modify: `frontend/src/stores/catalogue.ts` (the `load()` action, ~lines 148-162; add `markRaw` to the `vue` import)
- Test: `frontend/src/stores/__tests__/catalogue.test.ts`

**Interfaces:**
- Consumes: `getCatalogue` from `@/api/client` (unchanged).
- Produces: after `load()`, `this.catalogue` is a non-reactive (`markRaw`) array; `watched`/`ratings` maps still seeded from title fields.

- [ ] **Step 1: Write the failing test**

Add to `frontend/src/stores/__tests__/catalogue.test.ts`. Add `isReactive` to the vue import at the top:

```ts
import { isReactive } from 'vue'
```

Then add these tests inside `describe('catalogue store', ...)`:

```ts
  it('load() stores the catalogue non-reactively (markRaw)', async () => {
    const s = useCatalogueStore()
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue(fixtures)
    await s.load()
    expect(isReactive(s.catalogue)).toBe(false)
  })

  it('load() still seeds watched/ratings from title fields after markRaw', async () => {
    const seeded = [t({ id: 5, imdbId: 'tt5', watched: true, rating: 7 })]
    const s = useCatalogueStore()
    vi.spyOn(await import('@/api/client'), 'getCatalogue').mockResolvedValue(seeded)
    await s.load()
    expect(s.watched[5]).toBe(true)
    expect(s.ratings[5]).toBe(7)
  })
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && npm test -- catalogue`
Expected: FAIL on the `isReactive` assertion (currently the stored array is reactive → `true`).

- [ ] **Step 3: Implement `markRaw`**

In `frontend/src/stores/catalogue.ts`, add `markRaw` to the existing `vue` import (create the import if none exists):

```ts
import { markRaw } from 'vue'
```

Change the assignment in `load()`:

```ts
        this.catalogue = markRaw(await getCatalogue())
```

(Leave the watched/ratings seeding loop and everything else unchanged — it reads the same fields off the raw objects.)

- [ ] **Step 4: Run the store tests to verify pass**

Run: `cd frontend && npm test -- catalogue`
Expected: PASS (new tests green; existing `visibleTitles`/`genres`/`sort` tests still green — they assign `s.catalogue` directly and are unaffected).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/stores/catalogue.ts frontend/src/stores/__tests__/catalogue.test.ts
git commit -m "perf(store): store catalogue with markRaw to avoid ~20k reactive proxies"
```

---

## Task 3: `useDelayedFlag` composable

A boolean ref that becomes `true` only after its source stays truthy for `delayMs` (anti-flicker), and resets immediately when the source clears.

**Files:**
- Create: `frontend/src/composables/useDelayedFlag.ts`
- Test: `frontend/src/composables/__tests__/useDelayedFlag.test.ts`

**Interfaces:**
- Produces: `useDelayedFlag(source: Ref<boolean> | (() => boolean), delayMs: number): Ref<boolean>`. Must run inside a component `setup()` (it registers `onUnmounted`).

- [ ] **Step 1: Write the failing test**

Create `frontend/src/composables/__tests__/useDelayedFlag.test.ts`:

```ts
import { mount } from '@vue/test-utils'
import { describe, it, expect, vi, afterEach } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import { useDelayedFlag } from '@/composables/useDelayedFlag'

// The composable uses onUnmounted, so it must run inside a mounted component.
function harness(delay: number) {
  const source = ref(false)
  const cmp = defineComponent({
    setup() {
      const flag = useDelayedFlag(source, delay)
      return { flag }
    },
    render() {
      return h('span', this.flag ? 'on' : 'off')
    },
  })
  return { source, cmp }
}

afterEach(() => vi.useRealTimers())

describe('useDelayedFlag', () => {
  it('does not raise the flag before the delay elapses', async () => {
    vi.useFakeTimers()
    const { source, cmp } = harness(180)
    const w = mount(cmp)
    source.value = true
    await w.vm.$nextTick()
    vi.advanceTimersByTime(179)
    await w.vm.$nextTick()
    expect(w.text()).toBe('off')
  })

  it('raises the flag once the delay elapses', async () => {
    vi.useFakeTimers()
    const { source, cmp } = harness(180)
    const w = mount(cmp)
    source.value = true
    await w.vm.$nextTick()
    vi.advanceTimersByTime(180)
    await w.vm.$nextTick()
    expect(w.text()).toBe('on')
  })

  it('cancels the pending flag if the source clears before the delay', async () => {
    vi.useFakeTimers()
    const { source, cmp } = harness(180)
    const w = mount(cmp)
    source.value = true
    await w.vm.$nextTick()
    vi.advanceTimersByTime(100)
    source.value = false
    await w.vm.$nextTick()
    vi.advanceTimersByTime(200)
    await w.vm.$nextTick()
    expect(w.text()).toBe('off')
  })
})
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && npm test -- useDelayedFlag`
Expected: FAIL — module `@/composables/useDelayedFlag` does not exist.

- [ ] **Step 3: Implement the composable**

Create `frontend/src/composables/useDelayedFlag.ts`:

```ts
import { ref, watch, onUnmounted, type Ref } from 'vue'

/**
 * Returns a boolean ref that becomes true only after `source` has been
 * continuously truthy for `delayMs` (anti-flicker), and resets to false the
 * moment `source` becomes falsy. A pending raise is cancelled if the source
 * clears first. Must be called from a component setup().
 */
export function useDelayedFlag(
  source: Ref<boolean> | (() => boolean),
  delayMs: number,
): Ref<boolean> {
  const flag = ref(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  const getter = typeof source === 'function' ? source : () => source.value

  const clear = () => {
    if (timer !== undefined) {
      clearTimeout(timer)
      timer = undefined
    }
  }

  watch(
    getter,
    (active) => {
      if (active) {
        if (timer === undefined) {
          timer = setTimeout(() => {
            timer = undefined
            flag.value = true
          }, delayMs)
        }
      } else {
        clear()
        flag.value = false
      }
    },
    { immediate: true },
  )

  onUnmounted(clear)
  return flag
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd frontend && npm test -- useDelayedFlag`
Expected: PASS (all three cases).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/composables/useDelayedFlag.ts frontend/src/composables/__tests__/useDelayedFlag.test.ts
git commit -m "feat(ui): add useDelayedFlag composable for anti-flicker loading state"
```

---

## Task 4: Initial-load indicator in BrowseView

Show `ShimmerGrid` during the initial fetch past the 180ms threshold, and only show the empty-state text when the load is actually `ready`. Add an error branch so a failed load isn't a blank page (the current template has none).

**Files:**
- Modify: `frontend/src/views/BrowseView.vue`
- Test: `frontend/src/views/__tests__/BrowseView.test.ts`

**Interfaces:**
- Consumes: `useDelayedFlag` (Task 3); `useCatalogueStore` `status`/`error`/`resolving`/`visibleTitles`/`answerActive`/`load`.

- [ ] **Step 1: Write the failing tests**

Add to `frontend/src/views/__tests__/BrowseView.test.ts` (follow the file's existing mount/pinia setup; if it stubs child components, stub `ShimmerGrid` and `PosterGrid` the same way). Use fake timers to cross the 180ms threshold:

```ts
  it('shows the shimmer (not empty-state) while loading past the threshold', async () => {
    vi.useFakeTimers()
    const s = useCatalogueStore()
    s.status = 'loading'
    const w = mount(BrowseView, { global: { plugins: [/* pinia */] } })
    vi.advanceTimersByTime(180)
    await w.vm.$nextTick()
    expect(w.find('[data-test="shimmer"]').exists() || w.findComponent(ShimmerGrid).exists()).toBe(true)
    expect(w.find('.empty-state').exists()).toBe(false)
    vi.useRealTimers()
  })

  it('shows the empty-state only when ready with no matches', async () => {
    const s = useCatalogueStore()
    s.status = 'ready'
    s.catalogue = []
    const w = mount(BrowseView, { global: { plugins: [/* pinia */] } })
    await w.vm.$nextTick()
    expect(w.find('.empty-state').exists()).toBe(true)
  })
```

> Note for the implementer: match the existing test file's pinia/stub wiring exactly (it already mounts `BrowseView`). If `ShimmerGrid` is globally stubbed, assert on the stub; otherwise assert on `findComponent(ShimmerGrid)`. Import `ShimmerGrid` and `useCatalogueStore` if not already imported.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd frontend && npm test -- BrowseView`
Expected: FAIL — currently the shimmer only shows on `resolving`, and the empty-state renders during `loading`.

- [ ] **Step 3: Implement the template + script changes**

In `frontend/src/views/BrowseView.vue`, add to the `<script setup>` imports/logic:

```ts
import { useDelayedFlag } from '@/composables/useDelayedFlag'
```

Add after `const store = useCatalogueStore()`:

```ts
const showLoader = useDelayedFlag(() => store.status === 'loading', 180)
```

Replace the template block from `<ShimmerGrid v-if="store.resolving" />` through the empty-state `</div>` with:

```html
    <ShimmerGrid v-if="store.resolving || showLoader" data-test="shimmer" />
    <template v-else>
      <AnswerContext v-if="store.answerActive" />

      <PosterGrid
        v-if="store.visibleTitles.length > 0"
        :titles="store.visibleTitles"
        @select="openDetail"
        @find-similar="store.moreLike"
      />

      <div v-else-if="store.status === 'error'" class="empty-state" data-test="load-error">
        {{ store.error }} — <button class="retry-btn" @click="store.load()">Retry</button>
      </div>

      <div v-else-if="store.status === 'ready'" class="empty-state">
        {{ emptyCopy }}
      </div>
    </template>
```

Add a minimal `.retry-btn` rule to the `<style scoped>` block (reuse existing token vars):

```css
.retry-btn {
  background: none;
  border: none;
  color: var(--accent-text, #f5d24e);
  cursor: pointer;
  font: inherit;
  text-decoration: underline;
  padding: 0;
}
```

- [ ] **Step 4: Run the tests to verify pass**

Run: `cd frontend && npm test -- BrowseView`
Expected: PASS. Existing BrowseView tests stay green (the `PosterGrid` path is unchanged when `visibleTitles` is non-empty and `status` is `ready`).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/views/BrowseView.vue frontend/src/views/__tests__/BrowseView.test.ts
git commit -m "feat(browse): show shimmer during initial load; gate empty-state to ready; add error retry"
```

---

## Task 5: Debounce utility

A pure debounce so the header search doesn't trigger a 5k re-filter/re-sort per keystroke. Kept separate from the store so store tests stay synchronous.

**Files:**
- Create: `frontend/src/composables/debounce.ts`
- Test: `frontend/src/composables/__tests__/debounce.test.ts`

**Interfaces:**
- Produces: `debounce<A extends unknown[]>(fn: (...args: A) => void, delay: number): (...args: A) => void`.

- [ ] **Step 1: Write the failing test**

Create `frontend/src/composables/__tests__/debounce.test.ts`:

```ts
import { describe, it, expect, vi, afterEach } from 'vitest'
import { debounce } from '@/composables/debounce'

afterEach(() => vi.useRealTimers())

describe('debounce', () => {
  it('invokes only once after the delay with the latest args', () => {
    vi.useFakeTimers()
    const spy = vi.fn()
    const d = debounce(spy, 120)
    d('a'); d('b'); d('c')
    expect(spy).not.toHaveBeenCalled()
    vi.advanceTimersByTime(120)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith('c')
  })

  it('resets the timer on each call', () => {
    vi.useFakeTimers()
    const spy = vi.fn()
    const d = debounce(spy, 120)
    d('x')
    vi.advanceTimersByTime(100)
    d('y')
    vi.advanceTimersByTime(100)
    expect(spy).not.toHaveBeenCalled()
    vi.advanceTimersByTime(20)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith('y')
  })
})
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && npm test -- debounce`
Expected: FAIL — module `@/composables/debounce` does not exist.

- [ ] **Step 3: Implement the utility**

Create `frontend/src/composables/debounce.ts`:

```ts
/**
 * Returns a debounced wrapper of `fn`: calls are coalesced so `fn` runs once,
 * `delay` ms after the last call, with that call's arguments.
 */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  delay: number,
): (...args: A) => void {
  let timer: ReturnType<typeof setTimeout> | undefined
  return (...args: A) => {
    if (timer !== undefined) clearTimeout(timer)
    timer = setTimeout(() => {
      timer = undefined
      fn(...args)
    }, delay)
  }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd frontend && npm test -- debounce`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/composables/debounce.ts frontend/src/composables/__tests__/debounce.test.ts
git commit -m "feat(ui): add pure debounce utility"
```

---

## Task 6: Debounced search input in AppHeader

Bind the search box to a local ref (immediate echo) and push to `store.setQuery` debounced, so `visibleTitles` recomputes at most once per ~120ms.

**Files:**
- Modify: `frontend/src/components/AppHeader.vue`
- Test: `frontend/src/components/__tests__/AppHeader.test.ts`

**Interfaces:**
- Consumes: `debounce` (Task 5); `useCatalogueStore` `query`/`setQuery`.

- [ ] **Step 1: Write the failing test**

Add to `frontend/src/components/__tests__/AppHeader.test.ts` (match the file's existing mount/pinia setup):

```ts
  it('debounces search input into the store (no call until the delay)', async () => {
    vi.useFakeTimers()
    const s = useCatalogueStore()
    const spy = vi.spyOn(s, 'setQuery')
    const w = mount(AppHeader, { global: { plugins: [/* pinia */] } })
    await w.find('[data-test="search"]').setValue('alien')
    expect(spy).not.toHaveBeenCalled()
    vi.advanceTimersByTime(120)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith('alien')
    vi.useRealTimers()
  })
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && npm test -- AppHeader`
Expected: FAIL — currently `setValue` calls `setQuery` synchronously (the computed setter), so the "not called before delay" assertion fails.

- [ ] **Step 3: Implement the debounced binding**

Replace the `<script setup>` block in `frontend/src/components/AppHeader.vue` with:

```ts
import { ref, watch } from 'vue'
import { useCatalogueStore } from '@/stores/catalogue'
import { debounce } from '@/composables/debounce'

const store = useCatalogueStore()

// Local echo so the input stays instant; the store (and the 5k re-filter it
// drives) is updated on a debounce.
const query = ref(store.query)
const pushQuery = debounce((v: string) => store.setQuery(v), 120)
watch(query, (v) => pushQuery(v))

// Keep the box in sync if the query is reset elsewhere.
watch(() => store.query, (v) => { if (v !== query.value) query.value = v })
```

The template `v-model="query"` on the `[data-test="search"]` input is unchanged (it already binds to `query`).

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd frontend && npm test -- AppHeader`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/AppHeader.vue frontend/src/components/__tests__/AppHeader.test.ts
git commit -m "perf(header): debounce search input so visibleTitles recomputes at most ~1/120ms"
```

---

## Task 7: Full verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full frontend test suite**

Run: `cd frontend && npm test`
Expected: PASS — all suites green (target: the pre-existing 96+ plus the new tests).

- [ ] **Step 2: Type-check + build**

Run: `cd frontend && npm run build`
Expected: `vue-tsc -b` reports no type errors and `vite build` completes.

- [ ] **Step 3: Manual measurement checkpoint (operator, with the real ~5k library)**

Per the spec's "lighten first, then measure" decision, verify against the full catalogue:
- First paint has no long freeze; the shimmer appears (only if load > 180ms) and is replaced by posters.
- Scrolling top-to-bottom is smooth and the tab does not run out of memory.
- Search typing stays responsive.

Record the observed time-to-interactive and peak memory in `docs/superpowers/deferred-followups.md`. Only if still inadequate do we reconsider server-side pagination (which would then also need a "hydrate titles by ids" endpoint so Ask results can render).

- [ ] **Step 4: Commit any measurement notes**

```bash
git add docs/superpowers/deferred-followups.md
git commit -m "docs(followups): record catalogue perf measurements post-lightening"
```

---

## Self-Review

**Spec coverage:**
- Root cause #1 (render-all when unmeasured) → Task 1. ✓
- Root cause #2 (deep reactivity) → Task 2. ✓
- Change #3 (load indicator + threshold, reuse ShimmerGrid, gate empty-state) → Tasks 3 + 4. ✓
- Change #4 (debounced filter) → Tasks 5 + 6. ✓
- Testing section (computeWindow bounded, useDelayedFlag, store non-reactive, BrowseView states) → Tasks 1-4 tests. ✓
- Measurement checkpoint + pagination deferral → Task 7. ✓
- Error branch (blank-page gap noticed while gating the empty-state) → Task 4, a scoped improvement consistent with the spec's gating intent. ✓

**Placeholder scan:** No TBD/TODO; every code step shows full code. Test-wiring notes in Tasks 4/6 point the implementer at the existing file's pinia/stub pattern rather than guessing it — the assertions themselves are concrete.

**Type consistency:** `estimateRowHeight(containerWidth, cols)` used consistently (Task 1). `useDelayedFlag(source, delayMs)` defined Task 3, consumed Task 4 with a getter arg (a supported form). `debounce(fn, delay)` defined Task 5, consumed Task 6. `META_BLOCK` defined Task 1, imported in `computeWindow.ts`. `markRaw` from `vue` (Task 2). All names align.
