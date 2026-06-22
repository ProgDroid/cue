// ============================================================
// cue — design tokens (TypeScript module)
// For use in a Vue app: import { tokens } from '@/design/tokens'
// Mirrors tokens.css. Use whichever your stack prefers
// (CSS variables for styling, this object for logic/props).
// ============================================================

export const color = {
  bgApp: '#0b0c0f',
  bgPanel: '#0e0f13',
  surface1: '#15171c',
  surface2: '#181b21',
  surface3: '#1d2129',

  borderSubtle: 'rgba(255,255,255,0.07)',
  border: 'rgba(255,255,255,0.08)',
  borderStrong: 'rgba(255,255,255,0.12)',

  textStrong: '#f4f5f7',
  textPrimary: '#e9ebf0',
  textSecondary: '#c2c7d0',
  textTertiary: '#aab0bb',
  textMuted: '#8a909b',
  textFaint: '#5f6570',
  textFaintest: '#4f555f',

  accent: '#f5c518',
  accentText: '#f5d24e',
  accentOn: '#1a1400',
  accentTint: 'rgba(245,197,24,0.12)',
  accentTint2: 'rgba(245,197,24,0.14)',
  accentLine: 'rgba(245,197,24,0.20)',
  accentLine2: 'rgba(245,197,24,0.40)',
  accentFocus: 'rgba(245,197,24,0.50)',
} as const;

// Service identity. `label` is the display string; `dot` is the identity color.
// Render service as a small colored dot + label, NEVER a reproduced brand logo.
export const services = {
  plex:        { label: 'Plex',        dot: '#e5a00d' },
  disney:      { label: 'Disney+',     dot: '#2aa4ff' },
  crunchyroll: { label: 'Crunchyroll', dot: '#f47521' },
} as const;
export type ServiceKey = keyof typeof services;

export const font = {
  ui: "'Hanken Grotesk', system-ui, -apple-system, sans-serif",
  mono: "'JetBrains Mono', ui-monospace, monospace",
};

// type: [fontSize, fontWeight, letterSpacing?, lineHeight?]
export const type = {
  wordmark: { size: '21px', weight: 800, tracking: '-0.04em' },
  display:  { size: '38px', weight: 800, tracking: '-0.03em', leading: 1.05 }, // detail title
  section:  { size: '16px', weight: 700 },
  body:     { size: '15.5px', weight: 400, leading: 1.65 },                    // description
  ask:      { size: '14.5px', weight: 400 },                                   // ask bar input
  base:     { size: '13.5px', weight: 600 },                                   // controls, card title
  sm:       { size: '12.5px', weight: 500 },                                   // chips, buttons
  meta:     { size: '11px', weight: 600, mono: true },                         // year·type, ratings
  label:    { size: '10px', weight: 500, mono: true, tracking: '0.06em', upper: true }, // eyebrows
} as const;

export const space = { x1: 4, x2: 6, x3: 8, x4: 10, x5: 12, x6: 14, x7: 18, x8: 22, x9: 34 };
export const grid = { gapX: 18, gapY: 22, minPoster: { roomy: 196, balanced: 158, dense: 128 } };

export const radius = { sm: 6, md: 8, lg: 11, xl: 12, xxl: 16, pill: 999 };

export const shadow = {
  poster: '0 24px 50px rgba(0,0,0,0.55)',
  float: '0 24px 64px rgba(0,0,0,0.62)',
  orb: '0 10px 34px rgba(245,197,24,0.32), 0 4px 16px rgba(0,0,0,0.45)',
  switcher: '0 8px 30px rgba(0,0,0,0.50)',
  askbar: '0 6px 22px rgba(0,0,0,0.40)',
};

export const motion = {
  ease: 'cubic-bezier(0.4, 0, 0.2, 1)',
  hover: 160,  // ms — card lift / orb scale
  enter: 300,  // ms — fade-in
  panel: 220,  // ms — slide-in
};

export const layout = {
  headerH: 58,
  railW: 372,
  floatW: 388,
  posterAspect: '2 / 3',
};

// ------------------------------------------------------------
// Poster placeholder generator (use ONLY until real artwork is wired).
// Produces a cohesive per-title gradient from a hashed hue, plus a
// faint monogram. Replace with real poster <img> when available.
// ------------------------------------------------------------
export function posterHue(title: string): number {
  let h = 0;
  for (let i = 0; i < title.length; i++) h = (h * 31 + title.charCodeAt(i)) % 360;
  return h;
}

export function posterPlaceholder(title: string) {
  const h = posterHue(title);
  return {
    background: `linear-gradient(155deg, oklch(0.34 0.055 ${h}), oklch(0.16 0.04 ${h}))`,
    motif: `radial-gradient(circle, oklch(0.6 0.1 ${h}) 0%, transparent 70%)`,
    glyphColor: `oklch(0.82 0.07 ${h} / 0.16)`,
    backdrop: `linear-gradient(135deg, oklch(0.3 0.05 ${h}), oklch(0.14 0.035 ${h}))`,
  };
}

export function monogram(title: string): string {
  const skip = new Set(['a', 'an', 'the', 'of', 'x']);
  const words = title.replace(/[^a-zA-Z0-9 ]/g, ' ').split(' ').filter(w => w && !skip.has(w.toLowerCase()));
  if (!words.length) return title.slice(0, 2).toUpperCase();
  return words.slice(0, 2).map(w => w[0]).join('').toUpperCase();
}

export const tokens = { color, services, font, type, space, grid, radius, shadow, motion, layout };
export default tokens;
