# 隐私与遥测

Runner 绝不会在遥测中发送你的提示词、代码或会话记录，仅每天最多发送一次匿名心跳，用于按版本、操作系统、架构与基础功能使用情况统计活跃安装量。

## Runner 发送的内容

已签章的正式版与 nightly 构建每 24 小时最多向 `https://runnersh.dev/api/heartbeat` 发送一次 HTTPS `POST` 请求。未签章的本地开发构建绝不会发送心跳。

```json
{
  "install_id": "550e8400-e29b-41d4-a716-446655440000",
  "version": "0.20.0",
  "build_stamp": "20261126.1",
  "channel": "production",
  "os": "macos",
  "arch": "aarch64",
  "remote_enabled": true,
  "meta": {
    "active_runtimes": ["claude-code", "codex"],
    "used_mission_7d": true
  }
}
```

| 字段 | 说明 |
| --- | --- |
| `install_id` | 在本机生成的随机小写 UUID v4。关闭遥测后会立即删除，也可以随时重置。 |
| `version` | Runner 应用版本号。 |
| `build_stamp` | 官方正式版或 nightly 构建时写入的构建标识。 |
| `channel` | 发布通道：`production`、`nightly` 或 `preview-nightly`。 |
| `os` | 操作系统：`macos`、`windows` 或 `linux`。 |
| `arch` | CPU 架构：`aarch64` 或 `x86_64`。 |
| `remote_enabled` | 当前是否启用了远程访问（`true` 或 `false`）。 |
| `meta.active_runtimes` | 过去 7 天内在本机启动过的白名单编程智能体运行时键名（例如 `claude-code`、`codex`、`pi`）。 |
| `meta.used_mission_7d` | 过去 7 天内是否至少运行过一次多智能体任务（`true` 或 `false`）。 |
| `country`（边缘推导） | Cloudflare 边缘节点仅在内存中根据请求推导的两位国家/地区代码；不会存储或记录你的 IP 地址。 |

## Runner 绝不发送的内容

- 提示词、终端输入或输出，以及智能体对话内容。
- 源代码、文件内容、仓库名称或本地文件路径。
- 用户名、电子邮箱、主机名或身份验证凭据。
- IP 地址 —— 心跳接口绝不保存 IP 地址，也不会将其写入日志。
- 逐次点击埋点、会话录屏或第三方追踪 SDK。

## 数据保留与存储

心跳数据仅记录 UTC 日历日期（`YYYY-MM-DD`），存储在两张 Cloudflare D1 表中：`installs` 为每个 `install_id` 保留一条紧凑记录（包含 `first_seen_day`、`last_seen_day` 以及最新的版本、平台、国家/地区与功能元数据），`daily_pings` 为每个 `(day, install_id)` 最多保存一条包含当日心跳字段的记录。`daily_pings` 中超过 180 天的记录可定期清理，而 `installs` 保持每个安装一条记录。

## 对应用零影响

心跳在独立的后台线程中运行，总超时严格限制为 5 秒且禁用 HTTP 重定向，并与更新检查完全隔离。任何 DNS、TLS、代理、超时、重定向或 HTTP 错误都会静默丢弃，本次启动内不会重试，也不会弹出警告、提示条或阻塞应用启动与退出。

## 退出遥测与重置安装 ID

在全新安装时，Runner 会等待你完成或跳过首次引导后再发送第一次心跳；在首次引导中取消勾选**发送匿名每日安装计数**即可阻止任何初次发送。你也可以随时前往**设置 → 隐私**关闭遥测、查看实时发送的完整 JSON 数据，或点击**重置 ID** 轮换随机 UUID。关闭遥测会立即从本地 SQLite 数据库中删除 `telemetry_install_id`。

### 环境变量覆盖

在启动 Runner 的环境中设置 `DO_NOT_TRACK=1`（或 `true`）或 `RUNNER_TELEMETRY=0`（或 `false`）可无视已保存的设置强制锁定关闭遥测。由于后台 `runnerd` 守护进程仅在启动时读取这些变量，要让环境变量对已在运行的守护进程生效，需要先退出 Runner、执行 `runner daemon stop` 停止现有守护进程，再从已设置该变量的环境中重新启动应用（例如在 shell/会话中导出，或在 macOS 中运行 `DO_NOT_TRACK=1 open -a Runner`）。请注意，`runner daemon stop` 会终止仍在后台运行的活跃智能体会话（包括「保持运行」退出模式下留下的会话）—— 请先结束这些会话，或者直接在**设置 → 隐私**中关闭**匿名安装心跳**以立即退出遥测而无需停止任何会话。当环境变量覆盖生效时，**设置 → 隐私**会显示禁用遥测的环境变量名称，Runner 不会生成新的安装 ID，也不会发送心跳。

```sh
runner daemon stop
export DO_NOT_TRACK=1
```
