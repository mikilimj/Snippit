# Product

## Register

product

## Users

Gamers who clip their own highlights. They record whole sessions with an external
capture tool (GPU Screen Recorder, OBS, Steam), then come to ClipSnip afterwards
with a folder of long recordings and one job: get the twenty good seconds out and
into Discord.

Their context is post-session, at their own desk, usually late, usually with the
game still open behind the window. They are not editors and are not in an editing
mindset — they are in a *sharing* mindset, and the edit is friction between them
and the share. They repeat this often enough to learn shortcuts, but not often
enough to tolerate a tool that must be re-learned each time.

The job to be done: **find the clip, cut the clip, hear the right audio, send it.**
Everything else in the app exists to serve that line.

## Product Purpose

ClipSnip trims video clips that already exist on disk. It does not record. It sets
an in/out point, mixes the multiple audio tracks a game capture produces (game,
mic, Discord) live during preview, and exports — losslessly when nothing needs
re-encoding, or down to a size limit when a platform demands one. A SQLite-backed
library with folders, favorites, and watch-mode indexing keeps the clips findable
after the fact, and an optional rclone sidecar mirrors them to remote storage.

Success is measured in **time-to-share**, not in features used. A recording landing
on disk should reach a shareable file in under a minute, with the user never having
looked up how to do anything.

The thing that makes it worth building at all: the audio mixing. Every other trimmer
flattens a multi-track capture into whatever the muxer picked. ClipSnip lets you
hear each track, ride its level, and export exactly what you heard.

## Brand Personality

**Fast, precise, quiet.**

A workbench, not a stage. The clip is the only thing on screen with a right to be
loud; the interface around it is instrument-grade and recedes. Voice is short,
literal, and lowercase-in-spirit — it states what a control does and never
congratulates the user for using it. No exclamation marks, no "Awesome!", no
personality-as-copy. Confidence is expressed through precision (exact timecodes,
honest progress, real error text), not through decoration.

Feel to borrow from Raycast and Arc: **controls surface when they are relevant and
disappear when they are not.** The app should feel like it is anticipating the next
move rather than presenting every possible move at once.

## Anti-references

- **Generic SaaS dashboard.** Card grids, stat tiles, gradient hero buttons,
  rounded-everything, a web app cosplaying as a desktop app. This is a native tool.
- **Windows Movie Maker / iMovie.** Consumer-toy framing: oversized friendly
  buttons, wizard hand-holding, copy that explains what a video is.
- **Overloaded NLE.** Premiere's wall of tiny controls across twenty panels with
  nothing prioritized. Power without hierarchy is not power.
- **The AI-default dark theme.** Violet/indigo gradient on near-black, glassmorphic
  blur cards, animated aurora blobs. **The current ClipSnip theme is this**, and
  replacing it is in scope — the identity is being changed deliberately, not
  preserved. Whatever replaces it must not be reachable by guessing "dark tool for
  gamers".

## Design Principles

1. **The clip outranks the chrome.** Video and timeline get the pixels, the
   contrast, and the center. Every other region justifies its footprint or is
   collapsed, deferred, or removed. If a panel is visible while irrelevant, that is
   a bug.

2. **Surface on demand.** Sequence, music overlay, export-size, and format controls
   appear at the moment they apply. Progressive disclosure is the default; permanent
   visibility must be earned by permanent relevance.

3. **One library, not two.** Browsing clips and editing a clip are the same activity
   at different zoom levels, not two modes with two toolbars. Structure must reflect
   that; duplicated navigation is the symptom to eliminate.

4. **Reachable without the mouse.** The trim keys already exist (`I`/`O`); the rest
   of the workflow should match, and every shortcut should be discoverable in place
   rather than documented elsewhere. Keyboard is the fast path, not the fallback.

5. **Honest state.** Exporting shows real progress, failures show real error text,
   and lossless-vs-re-encode is visible before the user commits. Never imply work is
   done, safe, or reversible when it is not.

## Accessibility & Inclusion

Target **WCAG 2.2 AA**, keyboard-complete.

- Body text ≥4.5:1 against its surface; large text ≥3:1. The existing
  `--text-faint` (white at 40%) fails this and must be re-derived.
- A visible focus indicator on every interactive control. The current codebase has
  exactly one `focus-visible` rule across ~1,400 lines of CSS — this is the largest
  concrete accessibility gap.
- A complete keyboard path through find → trim → mix → export, with no
  mouse-only affordance (hover-reveal actions need a focus-reveal twin).
- `prefers-reduced-motion` alternative for every animation, including any
  ambient/background motion.
- Do not encode meaning in hue alone: in/out points, sync status, and error states
  need a shape, icon, or label alongside color. Assume red-green color blindness in
  the user base.
- Dark theme is the committed default (post-session, monitor-lit room). Any light
  variant is an addition, never a prerequisite for legibility.
