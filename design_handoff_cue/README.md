# Handoff: cue — personal media discovery

## Overview
`cue` is a self-hosted, dark-themed web app for browsing and discovering what to watch across a personal library that spans **Plex**, **Disney+**, and **Crunchyroll**. It is desktop-only and mouse-navigated. The defining idea is an **LLM ask experience integrated directly into browsing** — not a separate chatbot.

The product is two screens plus one integration pattern:
1. **Browse** (primary) — filterable poster grid with a persistent natural-language **ask bar**.
2. **Detail** — expanded title info, mark-as-watched, personal rating, and "similar in your library."
3. The **Integrated** ask pattern that ties them together (described in depth below).

## About the design files
The file in this bundle (`cue.dc.html`) is a **design reference created in HTML** — a working prototype showing the intended look and behavior. It is **not production code to copy**. Your task is to **recreate this design in the target Vue codebase** using its established patterns (components, state store, router, styling approach). If no environment exists yet, scaffold a standard Vue 3 + `<script setup>` + TypeScript app and build there.

The prototype uses a small in-house template runtime — **ignore the runtime mechanics**. What matters is the markup structure, the exact token values (captured in `tokens.css` / `tokens.ts`), and the interaction logic described here.

> The prototype includes a bottom **"chat concept" switcher** (Integrated / Unified / Side panel). That was an exploration aid for choosing a direction. **Ship only the "Integrated" concept** — drop the switcher and the other two layouts. They remain in the file for reference.

## Fidelity
**High-fidelity.** Colors, typography, spacing, radii, and interactions are final. Recreate the UI pixel-accurately using the tokens provided. The only deliberate placeholders are **poster images** (see Assets).

---

## Design tokens
Use `tokens.css` (CSS custom properties) for styling and/or `tokens.ts` (typed module) for logic. Highlights:

- **Surfaces** (cool near-black): app `#0b0c0f` · panel `#0e0f13` · surface `#15171c` · raised `#181b21` / `#1d2129`
- **Borders** (white alpha): `0.07` subtle · `0.08` default · `0.12` strong
- **Text**: strong `#f4f5f7` · body `#e9ebf0` · secondary `#c2c7d0` · tertiary `#aab0bb` · muted `#8a909b` · faint `#5f6570` · faintest `#4f555f`
- **Accent** (single amber): `#f5c518` fill · `#f5d24e` amber-on-dark text · `#1a1400` text on amber · tints `rgba(245,197,24, .12/.14/.20/.40/.50)`
- **Service dots**: Plex `#e5a00d` · Disney+ `#2aa4ff` · Crunchyroll `#f47521`
- **Type**: UI = **Hanken Grotesk** (400–800); metadata/labels = **JetBrains Mono** (400–600). Full scale in tokens.
- **Radii**: 6 / 8 / 11 / 12 / 16 / 999. **Poster aspect**: always `2 / 3`.
- **Motion**: hover lift 160ms, fade-in 300ms, panel slide 220ms; ease `cubic-bezier(.4,0,.2,1)`.

**Design principles to preserve:** minimal chrome, content-forward, single accent, monospace reserved strictly for metadata/labels (years, ratings, eyebrow labels), generous poster sizing, no rounded-corner-with-left-accent-bar tropes, no gradients except the subtle scrims/surfaces specified.

---

## Screens / Views

### 1. Browse (Integrated)
**Purpose:** the screen the user lives in — scan the catalogue, filter it, and ask for recommendations that reshape the very grid in front of them.

**Layout (top → bottom):**
- **Header** — fixed, `58px`, `padding: 0 22px`, `border-bottom: var(--border-subtle)`, bg `rgba(11,12,15,.92)` + `backdrop-filter: blur(8px)`.
  - Left: wordmark **`cue`** (21px / 800 / `-0.04em`, `#f4f5f7`) immediately followed by a `6px` amber dot. Then a mono eyebrow `SELF-HOSTED LIBRARY` (10px / `0.06em` / uppercase / `#4f555f`) separated by a `1px` left border with `14px` pad.
  - Center: **search** input, `max-width: 440px`, centered (`margin: 0 auto`), `height: 36px`, bg `#15171c`, border `var(--border)`, radius `8px`, leading `⌕` glyph at `12px` left, text `13.5px`. Focus: border `var(--accent-focus)`. Filters the grid by title substring, live.
  - Right: a `32px` circular avatar (`JD`, mono 12px, gradient `#2a2e37→#1a1d23`). *(In Integrated mode there is no header "Ask" button — the ask bar lives in the canvas.)*
- **Ask bar** — `position: sticky; top: 0` within the scrolling browse column; `padding: 18px 22px 12px`; sits on a `linear-gradient(180deg, #0b0c0f 76%, transparent)` so cards scroll cleanly under it.
  - The bar: `display:flex; align-items:center; gap:10px`, bg `linear-gradient(180deg,#181b21,#131519)`, border `var(--border-strong)`, radius `12px`, `padding: 9px 9px 9px 15px`, shadow `var(--shadow-askbar)`. Contents: amber `✦` (16px) · text input (placeholder *"Ask anything — “lighter after Frieren”, “short and tense”, “make me cry”"*, 14.5px) · amber **Ask** button (`height:34px; padding:0 16px`, bg `#f5c518`, text `#1a1400`, 13px / 700, radius 8px).
  - **Thread breadcrumb** (only when a conversation is active), `margin-top: 11px`, `display:flex; gap:6px; flex-wrap:wrap`: a mono `THREAD` eyebrow, then pill buttons. First pill **`Library`** (clears everything; transparent bg, border `0.12`, text `#aab0bb`). Then one pill per asked step; the **last/active** pill is amber-tinted (bg `rgba(245,197,24,.14)`, border `rgba(245,197,24,.4)`, text `#f5d24e`); intermediate pills are `#1d2129` / border `0.08` / `#c2c7d0`. Clicking a pill **steps back** to that point and truncates the thread after it.
- **Filter bar** — `padding: 18px 22px 14px`, `display:flex; flex-wrap:wrap; gap:10px 14px`. Composes WITH the ask (filters apply on top of an active answer set). Controls:
  - **Service** segmented group (wrapper bg `#15171c`, border `0.07`, radius `9px`, `3px` pad). Buttons: `All · Plex · Disney+ · Crunchyroll`. Each non-All button shows its `7px` service dot. Active button: bg `rgba(255,255,255,.08)`, text `#f4f5f7`; inactive text `#8a909b`. Radius `6px`.
  - **Type** segmented group, same styling: `All · Movies · Series`.
  - **Genre** `<select>` — appearance-none, `height:34px`, bg `#15171c`, border `0.07`, radius `9px`, mono 12px `#c2c7d0`, custom `▾` caret. Options: `All genres` + unique genres.
  - **Sort** `<select>`, same styling. Options: `Trending` (relevance/original order) · `Top rated` (IMDb desc) · `Newest` (year desc) · `A–Z`.
  - **Result count**, pushed right (`margin-left:auto`), mono 11px `#5f6570`, e.g. `12 titles`.
- **Answer context** (only when an answer is active) — `padding: 0 22px 6px`, fades in. Left: amber `✦` + the conversational **answer line** (14.5px `#e9ebf0`) and a mono sub-line (`{count} · refine or filter to narrow`, 10.5px uppercase `#5f6570`). Right: **refine chips** (`Even lighter · Make it shorter · Surprise me`) — `#1d2129` pills, border `0.10`, 12.5px `#d2d6dd`, hover border `rgba(245,197,24,.4)`.
- **Poster grid** — `padding: 6px 22px 40px`, `display:grid`, `grid-template-columns: repeat(auto-fill, minmax(158px, 1fr))` (balanced; roomy `196`, dense `128`), `gap: 22px 18px`.

**Poster card** (see component spec below). Clicking a card → Detail. Each card carries a small `✦` "find similar" button (top-right of its meta row) that asks *"More like {title}"* and reshapes the grid.

**Empty state:** centered `#5f6570` 14px line. Plain library-empty copy: *"Nothing in your library matches those filters."*; over-filtered-answer copy: *"Nothing in this result set matches those filters — loosen a filter or clear the thread."*

### 2. Detail
**Purpose:** decide on a single title; rate/mark it; discover adjacent titles.

**Layout:**
- **Backdrop band** — `height: 360px`, full-width, bg = the title's placeholder backdrop gradient, with a faint `200px` monogram pinned right, overlaid by two scrims: vertical (`transparent → #0b0c0f`) and horizontal (`#0b0c0f 8% → transparent 55%`) so text is legible left.
- **Back button** — top-left, `rgba(8,9,11,.6)` + blur, border `0.12`, radius 8px, label `← Library`.
- **Body** — `max-width: 1080px`, centered, pulled up over the backdrop (`margin-top: -180px`), `display:flex; gap:34px`.
  - **Left column (`232px`):** poster card (radius `12px`, shadow `var(--shadow-poster)`); **Mark as watched** button below (full width, `42px`; default = amber fill `#f5c518` / text `#1a1400` / label "Mark as watched"; watched = `rgba(245,197,24,.14)` fill, border `rgba(245,197,24,.4)`, text `#f5d24e`, label "✓ Watched"); then a **Your rating** well (`#15171c`, radius 10px) with 5 clickable `★` (filled `#f5c518`, empty `#3a3f4a`, 24px).
  - **Right column:** badge row — a service pill (dot + label) and an amber IMDb pill (`★ {rating} IMDb`). Then **title** `h1` (38px / 800 / `-0.03em` / lh 1.05 / `#f4f5f7`). Then a mono **fact line**: `{year} · {Movie|Series} · {runtime|eps} · {genres}` (12.5px `#8a909b`). Then **description** (15.5px / lh 1.65 / `#c2c7d0`, `max-width:600px`, `text-wrap:pretty`). Then **Cast** (mono eyebrow + name chips: `#15171c`, border `0.07`, radius 8px, 13px). Then **Similar titles available** (`in your library` mono note) — a `repeat(5,1fr)` grid of mini poster cards (same hover lift), computed by shared-genre overlap.

---

## Interactions & behavior

**The Integrated ask model (most important):**
- The ask bar is **always present** and **sticky** — never a modal, panel, or separate route.
- Submitting a query (Enter or Ask) resolves to a **result set + a one-line answer + a sub-line**, and **reshapes the same grid in place** (with a 300ms fade). It does NOT navigate away.
- **Chat and filters compose.** When an answer is active, the grid's base set = the answer's results; the service/type/genre/search/sort controls then filter/sort *within* it. (E.g. ask "cozy and low-stakes" → 5 results → switch service to Crunchyroll → narrows to the cozy Crunchyroll subset.)
- **Thread**: each ask (and each refine) pushes a step onto a breadcrumb. Clicking a step restores that step's result set and truncates later steps. **`Library`** clears the thread back to the full catalogue.
- **Refine chips** continue the conversation by transforming the *current answer set*: `Even lighter` (keep Comedy/Animation/Adventure/Romance/Musical), `Make it shorter` (sort by runtime/episode length asc), `Surprise me` (random IMDb ≥ 8 from the whole library).
- **Per-card `✦`** asks "More like {title}" → similar-by-shared-genre, reshaping the grid and adding a `≈ {title}` thread step.

**Other behavior:**
- **Hover**: poster cards lift `translateY(-4px)` over 160ms; orb/buttons scale or shift border to amber.
- **Mark as watched** toggles a flag; watched cards show a `22px` amber `✓` badge bottom-right of the poster.
- **Rating**: click a star to set 1–5; persists per title.
- **Card click** → Detail; scroll resets to top. **Back** → Browse, preserving filters/thread.
- **Keyboard**: `/` focuses the ask bar (when not already typing in a field); `Esc` clears the active answer + thread (or blurs the focused field). Implement as a global `keydown` listener.
- **Resolve shimmer**: while an ask resolves, the grid swaps to a shimmer skeleton (poster-aspect blocks pulsing via the `cuePulse` keyframe). The prototype fakes a ~450ms beat; **wire this to the real request's pending state** instead of a timer.
- **Over-filtered answer**: when filters narrow an active answer set to zero, the empty state reads *"Nothing in this result set matches those filters — loosen a filter or clear the thread."* (vs. the plain library-empty copy when no answer is active).

**Recommendation engine:** in the prototype this is rule-based (curated sets for the suggested prompts + a keyword fallback over the catalogue, plus "more like" by shared-genre overlap). **In production, replace with a real LLM call that is constrained to return only titles that exist in the user's catalogue.** Keep latency low — the experience must feel instant, never sluggish.

---

## State management
Suggested store (Pinia) shape:
- `catalogue: Title[]` — the merged library across services.
- **Filters:** `query`, `service ('all'|ServiceKey)`, `type ('all'|'movie'|'series')`, `genre ('all'|string)`, `sort ('trending'|'rating'|'year'|'az')`.
- **Answer:** `active: boolean`, `resultIds: number[]`, `line: string`, `sub: string`, `thread: { label, line, sub, ids }[]`.
- **User data:** `watched: Record<id, boolean>`, `ratings: Record<id, 1..5>`.
- **Navigation:** `screen ('browse'|'detail')`, `selectedId`.
- **Derived (computed):** `visibleTitles` = (answer.active ? resultIds : catalogue) → apply query/service/type/genre → sort. `similar(id)` = others sharing ≥1 genre, ranked by shared-count then rating, top 5.

`Title`: `{ id, title, year, service: ServiceKey, type: 'movie'|'series', genres: string[], imdb: number, len: string /* "164 min" | "28 eps" */, desc: string, cast: string[] }`. A seed catalogue of 28 titles lives in the prototype's logic class — reuse it for development.

---

## Assets
- **No bundled image assets.** Poster art is intentionally a **generated placeholder**: a per-title `oklch` gradient (hue hashed from the title) + a faint monogram. Use `posterPlaceholder()` / `monogram()` in `tokens.ts` until you wire real artwork from the Plex/Disney+/Crunchyroll metadata sources. When real posters exist, render an `<img>` at `2/3` and keep the placeholder as the loading/fallback state.
- **Service identity** is a **colored dot + text label**, never a reproduced brand logo — keep it that way to avoid trademark issues.
- **Icons** used are plain glyphs (`✦ ⌕ ★ ✓ ▾ ↑ ←`). Swap for your icon set if preferred; `✦` is the "ask/AI" mark throughout.
- **Fonts**: Hanken Grotesk + JetBrains Mono (Google Fonts). Self-host or link.

## Files
- `cue.dc.html` — the full interactive prototype (Browse + Detail + all three explored ask concepts; ship the **Integrated** one).
- `tokens.css` — CSS custom properties.
- `tokens.ts` — typed token module + poster-placeholder/monogram helpers.
