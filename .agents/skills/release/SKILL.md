---
name: release
description: Tag a new release, push, and publish on GitHub
---

# Release

Create a new versioned release for Runners.

## Steps

1. Ask the user for the version number (e.g. `0.3.0`) if not provided as an argument.

2. Bump version in all three files. **IMPORTANT: Do NOT use `sed` for version bumps.** Instead:
   - Confirm you're on `main` and the working tree is clean (`git status`). If not, stop and report.
   - Read each file first with the Read tool to confirm the current version string.
   - Use the Edit tool to replace the version in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`.
   - Run `cargo check` in `src-tauri/` to update `Cargo.lock`.
   - Stage the four changed files (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `Cargo.lock`) and commit with message `chore: bump version to v{version}`.
   - Push directly to `main`: `git push origin main`. No PR, no release branch.

3. Tag and push: `git tag -a v{version} -m "v{version}" && git push origin v{version}`

4. Wait for the release workflow to complete: `gh run list --workflow=release.yml --limit 1 --json status,conclusion,databaseId`. Re-poll until `status == "completed"` and `conclusion == "success"`. If it fails, stop and report.

5. Once the workflow succeeds, draft a release message by reviewing commits since the last tag: `git log $(git describe --tags --abbrev=0 HEAD^)..HEAD --oneline`

6. Categorize changes into sections: **What's New**, **Improvements**, **Bug Fixes** (omit empty sections).

7. Publish the release: `gh release edit v{version} --draft=false --notes "..."`. Include a **Download** section at the bottom with the `.dmg` filenames for Apple Silicon and Intel.

If any step fails, stop and report the error — do not continue.

## Notarization Commands

- **Check notarization history**:
  ```
  xcrun notarytool history --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID"
  ```

- **Check a specific submission**:
  ```
  xcrun notarytool info <submission-id> --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID"
  ```

- **Verify stapling on a DMG or .app**:
  ```
  stapler validate <file>
  ```

- **Check code signing**:
  ```
  codesign -dvv <path-to-app>
  ```

Note: Apple credentials are in `~/.zshrc`. The shell may not have them loaded — use literal values if env vars are empty.
