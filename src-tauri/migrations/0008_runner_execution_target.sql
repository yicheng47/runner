-- 0008: per-runner execution target for the Windows+WSL fork.
--
-- NULL / "wsl"  -> run the agent inside WSL (wrap the spawn in wsl.exe).
-- "native"      -> run the command directly on the Windows host (ConPTY),
--                  for Windows-native agents / shells (powershell, cmd,
--                  a Windows-installed claude/codex, Windows Python, …).
--
-- Ignored on macOS/Linux builds, where every spawn is already native.
-- NULL default keeps existing rows (and the seed crew) on the WSL path.
ALTER TABLE runners ADD COLUMN execution_target TEXT;
