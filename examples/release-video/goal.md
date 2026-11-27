# Release Video Crew — Mission Goal

A three-role product release video crew for short, code-driven feature demos (Remotion or similar). A director turns a feature spec into a tight storyboard and single-take recording plan, an editor builds and renders the composition with crisp captions and poster art, and a visual reviewer audits contact sheets and 1:1 crops for pacing, sharpness, and privacy redaction.

## Roles

Import [`release-video.crew.toml`](release-video.crew.toml) for all three roles and the production conventions:

- **@director** (Antigravity `gemini-4-neon`, `xhigh`) — writes the storyboard, opening diagram/poster concept, captions, and clean staging/recording checklist.
- **@editor** (Claude Code `claude-opus-5-5`, `xhigh`) — builds the Remotion composition, camera moves, captions, poster still, and contact-sheet exports.
- **@reviewer** (Codex `gpt-6.2-sol`, `high`) — inspects rendered contact sheets, 1:1 MP4 crops, frame 0 thumbnails, and every visible frame for timing defects, blur, or leaked personal info.

## Goal (replace this)

Name the feature or release, target length, languages, and footage location:

- *"Create a 45-second feature demo for v1.4 offline template import in `video/v1.4-templates/`. Source recording is at `public/footage/v1.4/take1.mp4`. Produce an animated cover diagram, standalone frame-0 poster, English captions, review MP4, and 5s + 1s transition contact sheets for visual QA. Stop before overwriting final release exports."*
