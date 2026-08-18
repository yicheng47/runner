# Runner Landing Page

## Status

In progress for issue [#235](https://github.com/yicheng47/runner/issues/235). The feature spec referenced by the issue (`docs/features/32-runner-landing-page.md`) was never written (Phase 1 "design the structure" was not done), so this impl doc is the primary record of the agreed approach.

## Problem

Runner's `/` route redirects straight to `/runners`. That's efficient for an installed desktop app, but it means there is no public product framing — nothing to point at for screenshots, demos, release pages, or a web preview. Runner needs a polished landing page that explains the product before a visitor downloads it.

Direction references (structure and product clarity, not copy/style to clone): [zcode.z.ai](https://zcode.z.ai/en) and [supacode.sh](https://supacode.sh/) — both are standalone marketing sites for the same category (native command center for coding agents): strong first viewport, direct macOS download CTA, real product screenshots, concise capability blocks, open-source trust signal.

## Key Decisions (this session)

The central call was *where and how* the landing lives. We rejected the issue's original framing (a `/` route inside the Tauri React app) in favor of a standalone static site. Rationale, in order of weight:

1. **Standalone public website, NOT a route inside the Tauri app.** A landing page is a different artifact, audience, and deploy target than the desktop app — it's served over HTTP, indexed, viewed in browsers before download. Embedding it as a `/` route in the app would (a) bundle a marketing page into the desktop binary, (b) force Tauri-vs-web branching on every route (the app routes call `invoke` and would break if the same build were served on the web), and (c) show a "Download" CTA to users who already installed. The desktop app's `/` stays exactly as-is (redirect to `/runners`).

2. **Same repo, separate `site/` build target (not a new repo).** The issue emphasizes *real* Runner screenshots. Keeping the site in this repo means screenshots (`assets/`) and the design system (Tailwind tokens) stay in sync with the actual app, with one home for PRs/issues/CI. A separate repo would drift and duplicate tokens. The site is a sibling build target — it does not ship inside the app bundle.

3. **Astro for the site.** A landing page is a *content-driven* site (mostly static content, sparse interactivity), which is Astro's sweet spot: it outputs static HTML and ships zero JS by default (fast first paint + good SEO with no prerender step), gives component DX + Tailwind, has first-class image optimization (`astro:assets`) for the screenshots, and supports interactive "islands" only where needed. Chosen over Vite+React→static (ships an SPA runtime and needs prerendering for a mostly-static page; the only real upside — reusing app components — isn't needed for marketing) and over fully hand-rolled HTML (less ergonomic for a rich, multi-section animated page). Visual polish (zcode-level animation, gradients, scroll effects) is CSS + motion and is independent of this choice.

4. **Fully static, no login, no backend.** Nothing dynamic; deploy is a folder of static files.

## Deviation from issue #235

The issue's written phases assume an in-app implementation: "standalone React page outside `AppShell`", "route `/` to the landing page", "wire external links through Tauri opener with browser fallback", and an "open-workspace" CTA. This impl supersedes that:

- Standalone static Astro site under `site/`, not a React route in the app.
- No Tauri opener / browser-fallback branching — plain anchor links on a web page.
- CTAs are **Download / Docs / GitHub** — no "open workspace," since web visitors don't have the app installed.

The issue body should be reconciled to this (or a short note added) so the record is consistent.

## Scope (v1)

- Top nav: Runner wordmark/icon, section links, GitHub, Download.
- Hero: product positioning, prominent macOS download CTA (→ GitHub releases latest), real product screenshot.
- Capability sections covering the product surfaces, each with real imagery from `assets/`: runners, crews, missions, terminals, event feeds, human asks.
- Trust/positioning blocks per the references (bring-your-own agents, parallel agent work, local-first, open source).
- Footer with Docs / GitHub / Download.
- Responsive desktop + mobile, no horizontal overflow.

## Out of scope (v1)

- Rendering the landing inside the desktop app (web-only).
- A docs site or blog (separate effort, later).
- Analytics, i18n, light/dark toggle (unless trivially free from shared tokens).
- Custom domain (start on the default host domain).

## Proposed Structure

- `site/` — Astro project.
  - `site/src/pages/index.astro` — the single landing page.
  - `site/src/components/*` — nav, hero, feature sections, footer.
  - Tailwind configured to reuse the app's design tokens (colors, fonts, radii) so the site matches Runner's visual system.
  - Images via `astro:assets`, sourced from the repo's existing `assets/` (`runner.png`, `crew.png`, `mission_feed.png`, `mission_terminal.png`, `icon.png`).
- Shared visual system: pull the same Tailwind theme variables the app uses (`src/index.css` tokens) rather than re-inventing colors/fonts.
- Deploy: static build of `site/` to GitHub Pages via an Actions workflow (default — in-repo, no new vendor), or Vercel pointed at `site/`. Decide in Phase 5.

## Implementation Phases

1. Scaffold `site/` Astro project; wire Tailwind with the app's tokens; import `assets/`.
2. Build the page: nav, hero (positioning + download CTA + screenshot), capability sections, footer.
3. Polish: responsive layouts (no horizontal overflow), motion/scroll reveals, image optimization, OG/social meta tags.
4. Links: Download → GitHub releases latest; Docs and GitHub links.
5. Deploy workflow (Pages/Vercel) + verify desktop and mobile screenshots.

## Open Questions

- Deploy host: GitHub Pages vs Vercel — default to Pages (in-repo Actions, no new vendor) unless a reason to prefer Vercel appears.
- Custom domain: deferred; default host domain for v1.
- Copy/positioning: draft from the zcode/supacode structure in Runner's own voice.

## References

- Issue [#235](https://github.com/yicheng47/runner/issues/235); [zcode.z.ai/en](https://zcode.z.ai/en); [supacode.sh](https://supacode.sh/).
- Screenshots already in repo: `assets/{runner,crew,mission_feed,mission_terminal,icon}.png`.
