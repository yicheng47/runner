# 566 — A refreshed README, in English and 简体中文

> Tracking issue: [#566](https://github.com/yicheng47/runner/issues/566)
> Priority: P2.

## Motivation

The README is Runner's front page on GitHub and the first thing a new user reads. Two things are wrong with it. It has no Chinese version, while the WeChat user group shows a growing share of users who read Chinese first; today the only 中文 on the page is the group's QR caption. And the English text has drifted behind the app: it was repositioned on 2026-09-09 and its Themes and Download lines were touched since, but it still describes the product as it stood in late August. An audit against the specs archived since the 0.6.0 cutover finds these gaps:

- Not mentioned at all: the terminal drawers under chats and missions (#469, #474); stop, resume and restart of a single mission slot (#542); mission permission modes with Bypass by default and no first-run prompt (#527, #541); the Settings → MCP catalog of every agent's servers (#555); the Settings → Skills pane (#73); `project_*` and `mission_set_project` on the MCP server (#554); opening terminal file links in your editor (#458); asking about a selection in a side thread (#511); app zoom from 60% to 200% (#549); the nightly channel and how to get on it (#502, #504, #505); the quit confirmation while work is running (#491); sessions outliving the app process (#466).
- Stale: the MCP feature card and its `assets/mcp_settings.png` describe the pane retired in #530, and the card's text says Settings → Agents registers the server while the catalog now lives in Settings → MCP; `crew.png`, `chat_split.png` and `multi_window.png` predate Runner Light and the current sidebar; "Also in the box" is four bullets for a much larger box.

## Scope

### In scope

- **`README.zh-CN.md`** at the repository root: a full 简体中文 rendering of `README.md` with the same section order, the same images, links and tables, and the same code and file names. Product terms stay as the app shows them (Runner, crew, mission, runner, Settings → Appearance, Sparkle, Nightly), in the style the 中文 half of the release notes already uses; once #565 ships an in-app 简体中文 glossary, the README follows it. The WeChat section moves up in the Chinese file, since it is the audience's contact point, and stays where it is in the English one.
- **Language switch.** One line under the title in both files, `English · 简体中文`, each linking to the other file. No flags, no badges.
- **English refresh.** Every gap in the audit list either gets a sentence in the feature table, a bullet in "Also in the box", or a deliberate skip recorded in the PR. The MCP card is rewritten in two halves: Settings → Agents registers Runner's own server with each agent, Settings → MCP catalogs every server each agent has; its screenshot is re-shot on the catalog. `crew.png`, `chat_split.png` and `multi_window.png` are re-shot on 0.8.7 in Carbon at the same window size as `mission_feed.png`. The Download section gains one paragraph on the nightly channel: what it is, where the `nightly` prerelease lives, and that it shares the release certificate.
- **Sync convention.** One line in `AGENTS.md` under Engineering Conventions: a change to `README.md` lands with the matching change to `README.zh-CN.md` in the same PR, and a PR that cannot translate marks the stale paragraph with `<!-- TODO zh-CN -->` rather than leaving it silently behind.

### Out of scope

- Localising the app's UI, which is #565.
- The landing page, #468, and any web copy.
- Rewriting `docs/`; the README links there and those documents have their own owners.
- A translation of `AGENTS.md` or the example crews' prompts.

## Implementation Phases

1. **English refresh.** The audit list into the README, the MCP card split, the four screenshots re-shot, the nightly paragraph. Ships on its own PR so the Chinese file translates a settled text.
2. **简体中文 README.** `README.zh-CN.md` and the language switch in both files.
3. **Convention.** The `AGENTS.md` line, in the same PR as phase 2.

## Verification

- [ ] Every issue in the audit list is either named in `README.md` or listed as skipped in the PR description.
- [ ] `assets/mcp_settings.png` shows the Settings → MCP catalog; `crew.png`, `chat_split.png` and `multi_window.png` show 0.8.7.
- [ ] `README.zh-CN.md` has the same headings in the same order as `README.md`, every image and link resolves, and every file name and issue number matches the English.
- [ ] Both files open with the language switch and each link lands on the other file on GitHub.
- [ ] `AGENTS.md` carries the sync convention.
