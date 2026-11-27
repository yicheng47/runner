# X Promotion Crew — Mission Goal

A bounded social-media discovery, drafting, and verified-reply crew for X. A browser-only scout finds fresh practitioner threads, an organizer selects high-signal finalists and enforces fact/disclosure rules, and a writer drafts concise replies in your voice — defaulting to human review before anything is posted.

## Roles

Import [`x-promotion.crew.toml`](x-promotion.crew.toml) for all three roles and the pipeline conventions:

- **@organizer** (Antigravity `gemini-4-neon`, `xhigh`) — owns the brief, candidate ranking, fact/disclosure checks, human approval gate, and final run summary.
- **@scout** (Codex `gpt-6.2-sol`, `medium`) — the only role that touches the browser; handles feed-first/search-second discovery, pre-post verification, and authorized posting.
- **@writer** (Antigravity `gemini-4-neon`, `xhigh`) — drafts concise replies in your voice and target language with weighted character counts; never opens the browser.

## Goal (replace this)

Fill in your topic lanes, documentation paths, style guide, and posting mode (`review` by default):

- *"Mode: review. Topic lanes: developer tooling, CLI workflows, and backend systems (English). Style guide: `docs/voice-guide.md`. Product docs: `README.md` (official site: `https://example.com`). Find 6–8 fresh threads from the last 3 hours, select 4 finalists (at most 1 with a transparent product mention), verify drafts, and present them in `runs/today.md` for my edits before posting anything."*
