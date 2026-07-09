# Rating Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `X+` external-rating threshold dropdown to the catalogue Browse filter bar that hides titles below the selected rating.

**Architecture:** Frontend-only. A pure `externalRating(title)` helper normalises each title to a unified 0–10 external rating (`score`, else `anilistScore / 10`, else `null`). The catalogue store gains a `minRating` state field, a `setMinRating` action, and one new filter step in the `visibleTitles` getter that drops unrated titles when a threshold is active. `FilterBar.vue` gains a fourth `<select>` bound to that state.

**Tech Stack:** Vue 3 + TypeScript + Pinia (options-store style), Vitest + @vue/test-utils.

## Global Constraints

- Client-side only — no backend endpoint, no `/api/catalogue` params, no migration.
- Personal ratings (`title.rating`) are out of scope; the filter uses external rating only.
- Follow existing store conventions: options-store getters/actions, `Title` type from `@/types`, test helper `t(p: Partial<Title>)`.
- Threshold values are exactly: `Any` (0), `6+`, `7+`, `8+`, `9+`.
- When a threshold is active, titles with no external rating are hidden.
- Run frontend tests from the `frontend/` directory.

---

### Task 1: `externalRating` helper + store filter

**Files:**
- Modify: `frontend/src/stores/catalogue.ts` (add export helper, `SortKey` unaffected, `State.minRating`, initial state, `visibleTitles` step, `setMinRating` action)
- Test: `frontend/src/stores/__tests__/catalogue.test.ts`

**Interfaces:**
- Produces: `export function externalRating(t: Title): number | null` — returns `t.score` if non-null, else `t.anilistScore / 10` if non-null, else `null`.
- Produces: store state `minRating: number` (default `0`), action `setMinRating(n: number): void`.
- Consumes: existing `Title` type, `visibleTitles` getter pipeline.

- [ ] **Step 1: Write the failing tests**

Add to `frontend/src/stores/__tests__/catalogue.test.ts`. First, import the helper by changing the existing import line:

```ts
import { useCatalogueStore, externalRating } from '../catalogue'
```

Then append these tests inside the `describe('catalogue store', ...)` block:

```ts
it('externalRating prefers score, falls back to anilistScore/10, else null', () => {
  expect(externalRating(t({ score: 8.5, anilistScore: 70 }))).toBe(8.5)
  expect(externalRating(t({ score: null, anilistScore: 85 }))).toBe(8.5)
  expect(externalRating(t({ score: null, anilistScore: null }))).toBeNull()
})

it('minRating=0 (default) applies no rating filter', () => {
  const s = useCatalogueStore()
  s.catalogue = fixtures
  expect(s.visibleTitles).toHaveLength(3)
})

it('minRating hides titles with no external rating', () => {
  const s = useCatalogueStore()
  s.catalogue = [...fixtures, t({ id: 4, title: 'Unrated', score: null, anilistScore: null })]
  s.setMinRating(8)
  expect(s.visibleTitles.map(x => x.id).sort()).toEqual([1, 2, 3]) // id 4 dropped
})

it('minRating compares AniList titles on the normalised 0-10 scale', () => {
  const s = useCatalogueStore()
  s.catalogue = [t({ id: 10, title: 'AL85', score: null, anilistScore: 85 })]
  s.setMinRating(8)
  expect(s.visibleTitles.map(x => x.id)).toEqual([10]) // 8.5 >= 8
  s.setMinRating(9)
  expect(s.visibleTitles).toHaveLength(0) // 8.5 < 9
})

it('minRating composes with the service filter', () => {
  const s = useCatalogueStore()
  s.catalogue = fixtures // 1:9.0 crunchyroll, 2:8.4 disney, 3:8.5 plex+disney
  s.setService('disney')
  s.setMinRating(8.5) // note: values in UI are whole, but getter must handle the threshold
  expect(s.visibleTitles.map(x => x.id).sort()).toEqual([3]) // 2 is 8.4 < 8.5, 1 not disney
})

it('minRating narrows an active Ask result', () => {
  const s = useCatalogueStore()
  s.catalogue = fixtures
  s.applyResult('q', { line: '', sub: '', ids: [1, 2, 3] })
  s.setMinRating(9)
  expect(s.visibleTitles.map(x => x.id)).toEqual([1]) // only the 9.0
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend && npx vitest run src/stores/__tests__/catalogue.test.ts`
Expected: FAIL — `externalRating` is not exported (import error) and `setMinRating` is not a function.

- [ ] **Step 3: Add the `externalRating` helper**

In `frontend/src/stores/catalogue.ts`, after the imports and before `type Status = ...`, add:

```ts
/** Unified best-available external rating on a 0–10 scale, or null if none. */
export function externalRating(t: Title): number | null {
  if (t.score != null) return t.score
  if (t.anilistScore != null) return t.anilistScore / 10
  return null
}
```

- [ ] **Step 4: Add `minRating` to state**

In the `State` interface, after `sort: SortKey`, add:

```ts
  minRating: number
```

In the `state: (): State => ({ ... })` initializer, after `sort: 'trending',`, add:

```ts
    minRating: 0,
```

- [ ] **Step 5: Add the filter step to `visibleTitles`**

In the `visibleTitles` getter, immediately after the genre filter line
`if (state.genre !== 'all') out = out.filter(t => t.genres.includes(state.genre))`, add:

```ts
      if (state.minRating > 0)
        out = out.filter(t => {
          const r = externalRating(t)
          return r != null && r >= state.minRating
        })
```

- [ ] **Step 6: Add the `setMinRating` action**

In `actions`, after `setSort(v: SortKey) { this.sort = v },`, add:

```ts
    setMinRating(n: number) { this.minRating = n },
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd frontend && npx vitest run src/stores/__tests__/catalogue.test.ts`
Expected: PASS — all existing and new tests green.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/stores/catalogue.ts frontend/src/stores/__tests__/catalogue.test.ts
git commit -F - <<'EOF'
feat(browse): add external-rating threshold to catalogue store

externalRating() normalises score / anilistScore into one 0-10 scale;
minRating state + visibleTitles step hides unrated titles when a
threshold is active. Composes with existing filters and Ask results.
EOF
```

---

### Task 2: Rating dropdown in `FilterBar`

**Files:**
- Modify: `frontend/src/components/FilterBar.vue` (add `ratingOptions`, `selectedRating` computed, `<select>` in template)
- Test: `frontend/src/components/__tests__/FilterBar.test.ts`

**Interfaces:**
- Consumes: `store.minRating` and `store.setMinRating(n)` from Task 1.
- Produces: a `<select data-test="rating-select">` with options `Any rating` / `6+` / `7+` / `8+` / `9+`.

- [ ] **Step 1: Write the failing tests**

Append to the `describe('FilterBar', ...)` block in `frontend/src/components/__tests__/FilterBar.test.ts`:

```ts
it('renders the rating threshold options', () => {
  const w = mount(FilterBar)
  const opts = w.get('[data-test="rating-select"]').findAll('option').map(o => o.text())
  expect(opts).toEqual(['Any rating', '6+', '7+', '8+', '9+'])
})

it('selecting 8+ sets store.minRating to 8', async () => {
  const w = mount(FilterBar)
  const s = useCatalogueStore()
  await w.get('[data-test="rating-select"]').setValue('8')
  expect(s.minRating).toBe(8)
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend && npx vitest run src/components/__tests__/FilterBar.test.ts`
Expected: FAIL — no element matches `[data-test="rating-select"]`.

- [ ] **Step 3: Add the rating options and binding to the script**

In `frontend/src/components/FilterBar.vue`, after the `sortOptions` array, add:

```ts
// Rating threshold options (value 0 = no filter)
const ratingOptions = [
  { value: 0, label: 'Any rating' },
  { value: 6, label: '6+' },
  { value: 7, label: '7+' },
  { value: 8, label: '8+' },
  { value: 9, label: '9+' },
]
```

After the `selectedSort` computed, add:

```ts
const selectedRating = computed({
  get: () => store.minRating,
  set: (v: number) => store.setMinRating(Number(v)),
})
```

- [ ] **Step 4: Add the `<select>` to the template**

In the template, immediately after the Sort `select-wrap` block (the `</div>` that closes the sort select) and before the `<!-- Result count -->` block, add:

```html
    <!-- Rating threshold select -->
    <div class="select-wrap">
      <select v-model="selectedRating" data-test="rating-select" class="filter-select">
        <option v-for="opt in ratingOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
      </select>
      <span class="select-arrow" aria-hidden="true">&#9660;</span>
    </div>
```

Note: `v-model.number` is not used because `option :value` binds numbers directly; the `set` coerces defensively via `Number(v)`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd frontend && npx vitest run src/components/__tests__/FilterBar.test.ts`
Expected: PASS.

- [ ] **Step 6: Run the full frontend suite + type-check**

Run: `cd frontend && npm run test && npm run type-check`
Expected: all tests pass; `vue-tsc` reports no errors.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/components/FilterBar.vue frontend/src/components/__tests__/FilterBar.test.ts
git commit -F - <<'EOF'
feat(browse): add rating threshold dropdown to FilterBar

Fourth select (Any/6+/7+/8+/9+) bound to store.minRating, styled like
the genre and sort selects.
EOF
```

---

## Self-Review

**Spec coverage:**
- Rating metric helper (`externalRating`, score→anilistScore/10 precedence) → Task 1 Steps 1, 3. ✓
- `minRating` state default 0 + `setMinRating` action → Task 1 Steps 4, 6. ✓
- Filter step after genre, hides unrated → Task 1 Step 5 + tests. ✓
- Composition with other filters + Ask results → Task 1 Step 1 tests. ✓
- Dropdown `Any/6+/7+/8+/9+` after Sort, styled like existing selects, `data-test="rating-select"` → Task 2. ✓
- Testing (store + component) → both tasks' Step 1. ✓
- Out of scope (personal rating, backend, half-steps) → nothing in plan touches these. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases" — every code step shows full code. ✓

**Type consistency:** `externalRating(t: Title): number | null`, `minRating: number`, `setMinRating(n: number)` used identically across both tasks and the store binding. `applyResult(label, { line, sub, ids })` matches the existing action signature (`AskResult`). ✓

## Verify (before finishing)

After both tasks, use the verify skill: run the app, open Browse, select `8+`, confirm the grid narrows to high-rated titles and the result count updates; select `Any rating` and confirm all titles return.
