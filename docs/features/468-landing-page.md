# Landing page for Runner

Tracking issue: [#468](https://github.com/yicheng47/runner/issues/468). Status: planned. Priority P2.

## Motivation

Runner has no web presence beyond the GitHub README. Anyone who hears about it lands on a repo page: a wall of markdown, a screenshot table, and a releases link three scrolls down. That works for developers who already want to read code. It does not work for the person a colleague sends a link to, who needs to understand in fifteen seconds what a *runner*, a *crew*, and a *mission* are and why they would want them.

The tool has also grown. Chats with split panes, missions with an event feed, MCP, multi-window, three runtimes. The README explains each feature well but explains the *model* nowhere near the top. The landing page's first job is user education: make the three-word vocabulary land before any feature does.

herdr.dev and onorca.dev show that this category has converged on a page shape: one-line promise, install or download CTA, a real product visual, a feature grid, an "agents we run" strip, social proof, footer. We follow the shape. The page earns its difference through the coordination model and the copy, not through a novel layout. This is also Jason's first landing page, so the plan favours the simplest tooling that produces a finished, deployable page over anything that teaches a framework.

## The page

One page, dark by default, desktop first with a single-column mobile layout. Content width 1120 px inside a 1440 canvas. Inter for text, the terminal font only inside terminal mockups. The accent is the app's Runner green (`$accent` in `design/runner.pen`), used for exactly one thing per section.

Sections, top to bottom:

1. **Nav.** App icon + "Runner". Links: Features, How it works, Crews, FAQ, GitHub. One button: **Download for Mac**.
2. **Hero.** Headline, two-line subheadline, primary CTA **Download for Mac** (points at `releases/latest`, label shows the current version), secondary **View on GitHub**. A fine-print line: `macOS 12+ · Apple Silicon · open source, GPL-3.0 · no account`. The visual is a real screenshot of a mission workspace: feed on the left, two slot terminals on the right, an `ask_human` card visible. A looping video replaces it once the README's planned demo recording exists; the layout reserves the same box.
3. **How it works.** The education section, immediately under the hero, three numbered cards in a row: **Runner → Crew → Mission.** Each card: the word, one sentence, one cropped screenshot (runner form, crew slot roster with the lead badge, mission feed). Under the row, one line on coordination: agents talk over an append-only event log with the bundled `runner` CLI; when a decision needs a human, `ask_human` surfaces in the feed. This is the only section that teaches vocabulary; everything below assumes it.
4. **Features.** Six cards, two rows of three, each with a title, two sentences, and a small screenshot or icon. Real terminals (PTY behind a GPU-drawn grid; TUIs render as themselves). Chats and split panes (no mission required; up to three panes per tab). Missions and the feed (persisted, replayable, survives quit). Human in the loop (`ask_human`, answer from the feed or type into any terminal). Drive it from your agents (MCP: an agent dispatches a crew and keeps working). Projects and windows (cwd-bound groups, `⇧⌘N` for a second screen).
5. **Works with.** A quiet strip: Claude Code and Codex first-class, TRAE on the same paths. Three logos or wordmarks, one honest sentence. No "29 agents detected" number; we don't have it and the page must not imply it.
6. **Crews you can copy.** The strongest proof that crews are a real idea: a card row of the example crews from `examples/`, each showing its handles as chips (`@coder` lead, `@reviewer`) and a one-line purpose. Peer coding (the default), dev crew, docs crew, then the fun ones: tic-tac-toe, werewolf, tomb raid. Each links to its folder on GitHub. Opening line: "A crew is a set of system prompts with handles. Paste one in and press Start."
7. **What Runner is not.** A short plain block instead of a comparison table: local only, your machine, your keys, no cloud, no account, no telemetry, macOS Apple Silicon only, open source under GPL-3.0. Story-shaped honesty is what has landed for Runner in community posts; comparison tables about competitors are what gets flagged.
8. **FAQ.** Six questions, collapsed: Is it free? Does it replace Claude Code or Codex? Do I have to use crews, or can I just run chats? Where does my data live? Intel, Linux, Windows? How does the MCP server fit?
9. **Final CTA.** Headline, **Download for Mac**, the same fine-print line, and a link to the releases page for the changelog.
10. **Footer.** GitHub, Releases, Docs (`docs/arch/arch.md`, `docs/product/vision.md`), Issues, License. `© 2026 wyc studios · v0.7.2`.

No star counts, install counts, or testimonials in v1. The numbers are small and testimonials we don't have; both references show them because they can. An empty social-proof section is worse than none.

## Copy

Hero headline, pick one in design review:

- *Your coding agents, working as a crew.*
- *Run a crew of coding agents from one desk.*
- *Claude Code and Codex, side by side, on one mission.*

Subheadline: *Runner is a native macOS app that runs CLI coding agents in real terminals, gives each one a role, and lets them coordinate on a goal — with you in the loop when it matters.*

How it works:

- **Runner.** *A configured agent: Claude Code or Codex, a role, a system prompt, a working directory. Reusable across crews.*
- **Crew.** *Runners in named slots with exactly one lead. Team conventions and a definition of done that every mission inherits.*
- **Mission.** *One goal, one live terminal per slot. The crew coordinates over a shared log; questions for you land in the feed.*

Final CTA headline: *Give your agents a desk to share.*

Copy rules carried over from the promotion playbook: concrete over hype, no "orchestrate" in headlines, every fact checked against the repo before publishing (license GPL-3.0-only, Apple Silicon only, three runtimes, current version).

## Tooling

- **Design first, in Pencil.** A new feature-scoped file, `design/landing.pen`, with two artboards: desktop 1440 and mobile 390. Screenshots go in as image fills from `design/landing/*.png`. The `runner.pen` tokens (colours, Inter, accent) are copied over so the page matches the app.
- **Hand-written HTML, CSS, and a few lines of JS.** No framework, no bundler, no npm. One `index.html`, one `styles.css`, one `main.js`. Herdr ships Astro; for one page that is a toolchain for its own sake, and hand-rolling is the most instructive first landing page. Pencil's HTML export is reference only and never shipped: exported markup is absolute-positioned and not responsive.
- **JS does two things.** Fetch the latest release tag from the GitHub API to fill the version label and download link, falling back to `releases/latest` and a baked-in version when the request fails or JS is off. Toggle FAQ items. Nothing else, no analytics in v1.
- **Assets.** Screenshots at 2× from the app in the Runner dark theme, exported to WebP with PNG fallback, each under 300 KB. Inter self-hosted as WOFF2 (the app already bundles Inter). Favicon from `assets/icon.png`.
- **Budget.** Under 1.5 MB total with images, no third-party requests, Lighthouse 95+ on all four scores.

## Repo and deployment

**Same repo, `site/` directory.** The page's screenshots, version, and copy track the product; a separate repo would drift the day after launch. `site/` is self-contained static files, so it never touches the Rust workspace or `make verify`.

**GitHub Pages through Actions.** A `pages.yml` workflow: on push to `main` that touches `site/**`, upload `site/` as the Pages artifact and deploy. Repo Settings → Pages → source "GitHub Actions". First address `https://yicheng47.github.io/runner/`. Free, HTTPS out of the box, no server, no account beyond GitHub. Cloudflare Pages or Vercel add nothing for one static page.

**Domain.** wyc studios owns `wycstudios.com`. Recommended: `runner.wycstudios.com`, one CNAME record to `yicheng47.github.io`, a `site/CNAME` file, "Enforce HTTPS" on. A dedicated `runner.*` domain is a later decision; the subdomain keeps the studio brand in the URL. Set the repo's homepage URL and add a Website link to the README once live.

**Builds and links stay decoupled from releases.** The download button resolves through `releases/latest`, so cutting a release never requires a site change. Only the baked-in fallback version and screenshots go stale, and both are edited in `site/` when the product visibly changes.

## Non-goals

- A docs site, blog, or changelog page. Docs stay in the repo; the changelog is the releases page.
- A Chinese version. Worth a follow-up given where Runner is promoted; v1 is English only.
- Analytics, newsletter capture, Discord. Nothing that needs a third party.
- Star counts and testimonials until they help.
- Nightly builds. The nightly channel is hidden by design and stays off the page.
- A comparison table against named competitors.

## Decisions

- **Shape follows the category, difference lives in section 3.** Visitors already know how to read this kind of page; spending novelty on layout costs comprehension. The Runner → Crew → Mission row is the one thing neither reference has and the one thing Runner most needs understood.
- **Hand-rolled over Astro.** One page, no content collections, no components to share. The maintenance cost of a build toolchain outweighs its benefit here, and the practice value is in the HTML and CSS.
- **Same repo, Pages, subdomain.** Zero new infrastructure, zero new accounts, one workflow file. Everything reversible.
- **Screenshot hero now, video later.** A screenshot is honest and ships this week; the demo video is already a README TODO and drops into the same box.
- **Honesty block instead of comparison.** Community history for Runner: story-shaped lands, launch-shaped gets flagged. The page inherits that rule.

## Implementation Phases

1. **Copy and assets.** Finalise the hero headline, section copy, and FAQ answers from this spec. Capture the six screenshots in the app at 2×.
2. **Design.** `design/landing.pen`: desktop and mobile artboards with real screenshots, all ten sections. Review before any HTML.
3. **Build.** `site/index.html`, `site/styles.css`, `site/main.js`, `site/assets/`. Semantic sections, one CSS file with custom properties for the tokens, a single breakpoint at 720 px. Version fetch with fallback. Verified against the design at 1440 and 390.
4. **Deploy.** `.github/workflows/pages.yml`, Pages source set to Actions, first deploy to `yicheng47.github.io/runner`. Then the CNAME and DNS for `runner.wycstudios.com`, HTTPS enforced, repo homepage and README link updated.

## Verification

- Every link resolves: download lands on the current DMG, GitHub links open the right folders, docs links open the right files.
- With JS disabled the page renders completely, the download link still works, and the baked-in version shows.
- Lighthouse 95+ on Performance, Accessibility, Best Practices, SEO at both viewports. HTML validates.
- Total transfer under 1.5 MB, no third-party requests in the network panel.
- Facts audit against the repo on the day of publishing: license, platforms, runtimes, version.
- Manual read by Jason: the three vocabulary cards are understandable without having used the app.
