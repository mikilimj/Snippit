# ClipSnip — Shell Redesign Design Brief

**Status:** **confirmed 2026-07-30 — implementation on hold pending go-ahead**
**Branch:** `feat/ux-shell-redesign` (forked from `grid-library-view`)
**Date:** 2026-07-30 · **Produced by:** `/impeccable shape`
**Anchors:** [`PRODUCT.md`](../../PRODUCT.md) (register: product)

---

## 1. Feature Summary

A full rework of the ClipSnip application shell — navigation, library browsing, and
the editor surface — plus the token layer underneath all of it. Today the app has
two parallel library UIs (a sidebar list and a grid mode) with duplicated brand rows
and toolbars, an editor column where trim, audio, sequence, music, and export
controls all compete at equal weight, and a visual identity that the product owner
has explicitly designated an anti-reference.

This redesign collapses the two library UIs into one, defers the controls that don't
apply to a simple trim, and replaces the "Midnight aurora" theme with an achromatic
instrument palette. Target user: a gamer who just finished a session and wants the
twenty good seconds in Discord in under a minute.

## 2. Primary User Action

**Find a clip, set in/out, and export — without leaving the keyboard or reading a
label twice.**

Everything else on screen is subordinate to that sentence. The audio-track mixer is
the single exception that earns permanent visibility: it is the reason the product
exists.

## 3. Design Direction

### Color strategy: Restrained

Product register floor, and correct here. One accent, used only for primary action,
current selection, trim range, and focus. Never decoration.

### Theme scene sentence

> It's 11pm, the session just ended, the room is lit only by the monitor, and the
> game is still running behind the window.

That forces dark. A second argument makes it non-negotiable and, more importantly,
sets the *chroma*: the user is looking at video. Any hue in the shell shifts how they
perceive the clip. So the shell is **achromatic — chroma exactly 0.000** — and the
video is the only saturated thing on screen. This is a functional decision, not a
style one, and it is the direct expression of Design Principle 1 ("the clip outranks
the chrome").

### Accent: amber

`oklch(0.80 0.15 80)`. Reads as instrument rather than app — VU meter, tape deck,
hardware. Chosen over rose specifically so that **red stays free for destructive
actions**, which matters because delete-from-library and delete-from-server are both
one click away in the clip actions.

### Anchor references

| Reference | What is being borrowed | What is *not* |
|---|---|---|
| **Raycast** | Controls surface on demand and vanish when irrelevant; keyboard as the primary path, discoverable in place | Not the command-palette-as-only-navigation model — search is a field, not a modal |
| **Vercel / MUBI** | Achromatic near-black shell, flat and matte, depth from spacing not shadow | Not the marketing gloss; this is a tool |
| **DaVinci Resolve** | Neutral surround as a *color-fidelity* requirement around video; tabular timecode | Not the panel density or the twenty-toolbar wall — explicitly an anti-reference |

### Category-reflex check

- **First-order** — "dark tool for gamers" predicts violet/indigo gradient on
  near-black with glass cards. That is the current theme, and it is being deleted.
- **Second-order** — "gaming video tool that *isn't* AI-purple" predicts either
  terminal-green-on-black or the mid-gray NLE. Both rejected: green-on-black is the
  terminal reflex, and mid-gray was offered and declined in discovery. The achromatic
  shell survives because it has a reason that is not aesthetic (video perception),
  and the amber accent is not reachable by guessing the category.

## 4. Scope

- **Fidelity:** production-ready. Implementable straight through, no gaps left to
  build-time judgment.
- **Breadth:** whole surface — shell, library, editor, settings, first-run, plus the
  token layer.
- **Interactivity:** shipped-quality. Every component gets default, hover, focus,
  active, disabled, loading, and error where applicable.
- **Time intent:** polish until it ships.
- **Out of scope:** the audio engine, export logic, and existing `api.ts` command
  signatures. This is a frontend shell change with **one deliberate backend
  exception**: the recorded-at column needed for date sorting (§11.1).

## 5. Layout Strategy

### The core structural move: one library, not two

Today: `viewMode: "sidebar" | "grid"` swaps between two complete library UIs, each
with its own brand row and its own toolbar. Both render the same clips.

After: **there is exactly one list of clips — the grid — and the sidebar stops being
a second copy of it.**

```
┌──────────────────────────────────────────────────────────────────────┐
│  ClipSnip   [ search clips…              ⌘K ]   sort ▾    ⟳    ⚙     │  top bar, 48px
├──────────┬───────────────────────────────────────────────────────────┤
│ All      │                                                           │
│ Favorite │                     S T A G E                             │
│ Unsorted │       grid of clips  ─or─  the open clip's editor         │
│ ──────   │                                                           │
│ Clutch   │                                                           │
│ Funny    │                                                           │
│ + New    │                                                           │
├──────────┤                                                           │
│ ● Synced │                                                           │
└──────────┴───────────────────────────────────────────────────────────┘
   nav rail                            one surface, two contents
   180px, collapsible
```

- **Top bar (48px).** Wordmark, search field, sort control, rescan, settings. This is
  the *only* toolbar in the app — the duplicated `brand-row` + `grid-toolbar` +
  `library-header-actions` all collapse into it.
- **Nav rail (180px, collapsible to 0).** Saved views and folders only. It never
  lists clips. This is what makes the duplication structurally impossible to
  reintroduce.
- **Stage.** Renders either the clip grid or the editor. Opening a clip replaces the
  grid; `Esc` returns to it with scroll position and selection intact. No mode
  toggle, no "← Editor" button, because there are no longer two modes to switch
  between — there is one surface showing one of two things.

### Editor composition

```
┌─────────────────────────────────────────────┬──────────────────┐
│  clipname.mkv   1920×1080 · h264 · 2 tracks │  AUDIO TRACKS    │
│                                        Esc ×│  ─────────────   │
├─────────────────────────────────────────────┤  Track 1  [====] │
│                                             │  ☑ game    100%  │
│              V I D E O                      │                  │
│         on --stage (L 0.04)                 │  Track 2  [==  ] │
│                                             │  ☑ mic      60%  │
│                                             │                  │
├─────────────────────────────────────────────┤  ┄┄┄┄┄┄┄┄┄┄┄┄┄┄  │
│  ▶  0:04.2      ├───█████████────┤   0:38.1 │  + Sequence      │
│                  in 0:12.0  out 0:21.4      │  + Music         │
│                  selection 9.4s             │  (collapsed)     │
├─────────────────────────────────────────────┴──────────────────┤
│                                          [ Export ⌄ ]  Ctrl+E  │
└────────────────────────────────────────────────────────────────┘
```

- Video gets the pixels and sits on the darkest token (`--stage`, L 0.04) so nothing
  in the chrome competes with it.
- Timeline grows from a thin bar into a real scrub track with **draggable** in/out
  handles (today they are display-only — set by `I`/`O`, not movable, which is a
  functional gap, not just a visual one).
- Audio tracks hold a permanent right column. Confirmed in discovery as the one panel
  that stays.
- Sequence and Music are collapsed affordances until used.
- Export is a single primary action; size and format live in its popover, revealed at
  commit time.

### Rhythm

Spacing scale `4 / 8 / 12 / 16 / 24 / 32 / 48`. Density varies deliberately: the nav
rail and top bar run tight (8–12), the stage runs open (24–32). No uniform padding
across regions — that flatness is what makes an interface read as a template.

## 6. Design System

### Tokens — all values verified, not asserted

Contrast computed via OKLCH → linear sRGB → WCAG relative luminance.

**Surfaces** (chroma exactly 0.000 — the whole point):

| Token | OKLCH | Hex | Role |
|---|---|---|---|
| `--stage` | `0.04 0 0` | `#000000` | video letterbox, darkest |
| `--bg` | `0.14 0 0` | `#090909` | window base |
| `--surface` | `0.18 0 0` | `#121212` | rails, panels, top bar |
| `--surface-raised` | `0.23 0 0` | `#1d1d1d` | inputs, cards, popovers |
| `--surface-hover` | `0.27 0 0` | `#262626` | hover fill |

**Lines:**

| Token | OKLCH | Hex | Role |
|---|---|---|---|
| `--border` | `0.30 0 0` | `#2e2e2e` | decorative hairlines only |
| `--border-control` | `0.52 0 0` | `#6f6f6f` | **form-control boundaries** |

`--border-control` is set at exactly the lightness that clears WCAG 2.2 §1.4.11
non-text contrast against every surface it can appear on: **3.61:1** on `--bg`,
**3.41:1** on `--surface`, **3.07:1** on `--surface-raised`. Anything lighter is
unnecessary; anything darker fails on raised surfaces. The current `--border-strong`
(white at 14%) is nowhere near this.

**Ink:**

| Token | OKLCH | Hex | vs `--surface` | vs `--surface-raised` |
|---|---|---|---|---|
| `--text` | `0.97 0 0` | `#f5f5f5` | 17.24:1 | 15.49:1 |
| `--text-secondary` | `0.78 0 0` | `#b7b7b7` | 9.40:1 | 8.44:1 |
| `--text-muted` | `0.66 0 0` | `#929292` | 6.04:1 | 5.43:1 |

The muted tier passes AA body text on every surface. **This is the fix for the
single largest legibility problem in the current theme**: `--text-faint` is
`rgba(255,255,255,0.4)`, which fails 4.5:1 outright and is currently used for
`.library-meta`, `.folder-new`, and placeholder text.

**Accent & semantic:**

| Token | OKLCH | Hex | Contrast | Role |
|---|---|---|---|---|
| `--accent` | `0.80 0.15 80` | `#f0b135` | 9.92:1 on surface | primary action, selection, trim range, focus |
| `--accent-dim` | `0.70 0.13 80` | `#c8942d` | 6.94:1 on surface | hover/pressed on accent surfaces |
| `--on-accent` | `0.18 0.03 80` | `#181003` | 9.93:1 on accent | text on amber fills |
| `--danger` | `0.68 0.17 27` | `#ef675c` | 6.05:1 on surface | destructive only |

**Deleted outright:** `--grad-primary`, `--grad-shadow` (gradient buttons are in the
anti-references), `--panel-glass` (glassmorphism), `--in-color` / `--out-color` (see
below), `--accent-tint` / `--accent-border` in their violet form, and the entire
`.aurora-blob` system including its three keyframe animations.

**Also:** `index.html` carries a hardcoded `<style>html { background: #06060a }</style>`
that must move to `--bg`, or the window flashes the old violet-black on every launch.

### Trim markers without hue-coding

`--in-color` (teal) and `--out-color` (rose) are removed. The trim **range** is
filled with `--accent` at 20% — it is the selection, and selection is what the accent
is for. The two handles are distinguished by **shape, not hue**: the in-handle is a
left-facing bracket, the out-handle right-facing, each carrying its `I`/`O` label.

This satisfies the PRODUCT.md requirement that meaning never rides on hue alone, and
it removes two colors from a palette that is trying to stay achromatic.

### Typography

One family: **Schibsted Grotesk Variable**, already bundled — no new dependency, and
the product register notes a single well-tuned sans is usually right for app UI.

Fixed rem scale, ratio ~1.15 (no `clamp()` — fluid type is a brand-register tool and
actively harms a resizable desktop panel):

| Step | Size | Use |
|---|---|---|
| `--fs-micro` | 11px / 600 / +0.02em | rail section labels |
| `--fs-small` | 12px | metadata, timecodes, helper text |
| `--fs-base` | 13px | body, labels, buttons |
| `--fs-md` | 15px | clip title, panel headings |
| `--fs-lg` | 19px | modal titles |
| `--fs-xl` | 25px | empty-state heading (the only large type in the app) |

**All numerics get `font-variant-numeric: tabular-nums`** — timecodes, durations,
resolutions, percentages, file sizes. A playhead readout whose digits shift width as
it runs is the difference between an instrument and a web page. This is the single
highest-leverage typographic change in the redesign.

### Icons

Every icon today is a Unicode glyph — `⧉ ⟳ ⚙ ★ ☆ ♪ ×` — which renders differently on
every platform, cannot be sized or aligned reliably, and carries no accessible name.
Replace with a local `Icon.tsx`: inline SVG, 16px grid, 1.5px stroke, `currentColor`,
`aria-hidden` with the label on the control. ~18 icons needed (search, sort, refresh,
settings, star, folder, folder-plus, trash, cloud, cloud-off, play, pause, scissors,
music, plus, close, chevron, grid). No dependency — a Tauri bundle should not pull an
icon package for eighteen paths.

### Component states

Every interactive component ships all seven: default, hover, focus, active, disabled,
loading, error. The current codebase ships roughly two.

**Focus is the flagship fix.** There is exactly **one** `focus-visible` rule across
~1,400 lines of CSS. The redesign defines one global token —
`--focus-ring: 0 0 0 2px var(--bg), 0 0 0 4px var(--accent)` — applied to every
focusable element, with no `outline: none` anywhere that isn't immediately replaced.
Note `.export-size-select:focus` and `.storage-field input:focus` currently set
`outline: none` with only a border-color change, which is not a sufficient indicator.

**Off-palette cleanup.** `.export-size-select` / `.export-size-custom` still carry
`#3a3a3a`, `#2a2a2a`, `#d0d0d0`, `#666`, and a stray blue `#4a8fc0` from a pre-theme
era. These are the most visibly inconsistent controls in the app and get rebuilt on
the shared form-control vocabulary.

## 7. Key States

| Surface | State | What the user sees |
|---|---|---|
| **Library grid** | first run, empty | Not "nothing here." Teaches the interface: "ClipSnip trims recordings you already have. Point it at your capture folder and clips appear here automatically." + `Choose folder` + `Open a clip` |
| | empty *filter* | Distinct per view — favorites, unsorted, and empty folder each say something specific and offer a way out of the filter |
| | search, no match | Echoes the term, offers to clear: "No clips match `push`." |
| | loading | Skeleton tiles at true thumbnail aspect ratio — never a centered spinner |
| | thumbnail missing | Neutral placeholder with a film-frame mark, not a blank box |
| | populated | Virtualized grid, `repeat(auto-fill, minmax(200px, 1fr))` |
| **Clip card** | default / hover / focus | Actions are hover-revealed today, which makes them keyboard-unreachable. Every hover-reveal gets a `:focus-within` twin |
| | selected | Amber ring, not a background wash |
| | syncing / synced / sync error | Icon + text, never a bare colored dot |
| | renaming | Inline input, Enter commits, Esc cancels |
| **Editor** | loading meta | Skeleton in the info bar; stage holds aspect ratio so nothing reflows |
| | proxy generating | Inline, non-blocking: "Building a preview for this format…" with progress |
| | audio preparing | Per-track skeleton in the right column; video stays playable |
| | unplayable codec | Real error text plus the actual fallback taken |
| | no in/out set | Export is enabled and exports the whole clip; the timeline hints `I` / `O` |
| **Export** | idle / confirming | Popover: size target, format, and **an explicit lossless-vs-re-encode readout** — Principle 5 |
| | in progress | Determinate bar, percent, elapsed, cancel |
| | success | Inline confirmation with `Show in folder` / `Copy path`. Not a modal, not a toast that vanishes before it's read |
| | failure | Real ffmpeg error text, retry, and a copy-details action |

## 8. Interaction Model

### Keyboard map — complete path, no mouse required

| Key | Action |
|---|---|
| `Ctrl/Cmd+K` or `/` | Focus search |
| `Esc` | Editor → library · clear search · close popover |
| `↑ ↓ ← →` | Navigate grid (roving tabindex) |
| `Enter` | Open focused clip |
| `F` | Toggle favorite on focused clip |
| `Space` | Play / pause |
| `I` / `O` | Set in / out *(exists today)* |
| `Shift+I` / `Shift+O` | Clear in / out |
| `← →` | Seek ±1s · `Shift` ±10s |
| `,` / `.` | Frame step |
| `Home` / `End` | Jump to in / out |
| `[` / `]` | Previous / next clip in current view |
| `Ctrl+E` | Export |
| `?` | Shortcut sheet |

Today only `I` and `O` are bound globally. Discoverability is in-place: shortcut hints
render inside the controls they belong to, not in a help document.

### Accessibility work items

1. Library items are `<li onClick>` — not focusable, not activatable by keyboard. They
   become real buttons / a roving-tabindex grid.
2. The timeline is pointer-only with no ARIA. It becomes `role="slider"` with
   `aria-valuenow/min/max/valuetext` (spoken as timecode), arrow-key operable, and the
   in/out handles become separately focusable sliders — which also delivers the
   drag-handles gap noted in §5.
3. Every hover-reveal action gains a focus-reveal twin.
4. `prefers-reduced-motion` alternative on every transition. The aurora animations are
   deleted, which removes the largest ambient-motion source outright.
5. Sync status, trim points, and error states carry icon or text alongside color.

### Motion

150–200ms, `ease-out-quart`. Motion conveys state only.

- Disclosure (sequence, music, export popover): height + opacity, 180ms.
- Grid card hover: thumbnail brightness and border, **not** a lift — the translate-and-
  shadow card hover is the SaaS tell in the anti-references.
- Playhead: driven by the existing rAF loop, untouched.
- Export progress: determinate, real.
- No page-load choreography. The app loads into a task.

## 9. Content Requirements

Voice per PRODUCT.md: short, literal, no exclamation marks, no congratulation.

- **Rewrite:** `"Open a clip, trim it, share it — in seconds."` — the marketing line
  in the empty state. Empty states teach the interface; they don't pitch it.
- **Rewrite:** `"No favorites yet — tap ☆ on a clip."` — "tap" is touch language in a
  desktop app, and the glyph won't survive the icon migration.
- **Add:** search-empty, sync-error, export-success, export-failure, and
  lossless-vs-re-encode copy — none exist today.
- **Ranges to design against:** 0 / ~40 / 1000+ clips; filenames from 8 to 120
  characters (grid cards need a two-line clamp with a full-name tooltip); durations
  from 2s to 2h; 1–4 audio tracks; folder names up to ~30 characters.
- **Assets:** the existing `logo.png` is the only image asset; everything else is
  icons (inline SVG) and user thumbnails. Nothing new to generate.

## 10. Implementation Order

Six phases, each independently shippable and reviewable.

1. **Token layer** — rewrite `theme.css`, fix `index.html`, delete gradient/glass/
   aurora tokens. Nothing moves structurally; the app just changes color.
2. **Primitives** — `Icon.tsx`, button/input/select vocabulary, global focus ring,
   rebuild the off-palette `export-size` controls.
3. **Shell** — top bar, nav rail, stage. Delete `viewMode`, merge the sidebar list and
   `LibraryGrid` into one grid. This is the largest single change.
4. **Library** — search, sort, virtualization, card states, keyboard grid navigation.
   Includes the one backend change in scope: the recorded-at column in `library.rs`
   plus its migration/backfill (§11.1), landing before the sort control that needs it.
5. **Editor** — timeline rebuild (draggable + ARIA slider), transport row, audio
   column, progressive disclosure for sequence and music, export popover.
6. **Polish** — motion, remaining states, copy pass, full keyboard audit.

Existing pure-logic tests (`libraryView`, `sequence`, `trim`, `exportOptions`,
`resizable`, `preview`, `audioChunks`) are unaffected; new logic — search/sort
filtering, grid virtualization windowing, keyboard-map resolution — arrives with
co-located tests per the project's conventions.

## 11. Resolved Questions

Both open questions were decided at confirmation time:

1. **Sort by date — RESOLVED: in scope.** `ClipRow` exposes no timestamp, so
   "newest first" — the correct default for a clip library — is not currently
   expressible. A recorded-at column is added to the SQLite index in `library.rs`,
   populated from filesystem `mtime` at scan and watch time, surfaced through
   `ClipRow`, and made the default library sort. This is the **one deliberate
   exception to the "frontend only" scope in §4**, and it lands inside this redesign
   rather than on a separate branch.

   Requires: a schema migration for existing libraries (backfill `mtime` for already-
   indexed clips on first run after upgrade, rather than forcing a full rescan).

2. **Search backend — RESOLVED: client-side.** Filtering runs over the already-loaded
   rows; no Rust change. Correct through the low thousands, which covers the stated
   range. Revisit only if real libraries outgrow it.

Nothing in this brief is unresolved.
