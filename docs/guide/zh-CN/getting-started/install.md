# 安装 Runner

安装 Runner 并登录智能体 CLI，即可开始工作。Runner 提供工作区和终端；智能体 CLI 提供模型连接，并使用自己的账号。

## 下载应用

从 [Runner 下载页](https://runnersh.dev/zh-CN/downloads/)或 [GitHub Releases](https://github.com/yicheng47/runner/releases/latest) 下载最新稳定版。

| 平台 | 安装包 | 系统要求 |
| --- | --- | --- |
| macOS | 已签名并公证的 `…-arm64.dmg` 或 `…-x64.dmg` | macOS 12 或更高版本，Apple Silicon 或 Intel Mac |
| Windows | 已签名的 `Runner-Setup-…-x64.exe` | Windows 10 1809 或更高版本，x64 |
| Linux（测试版） | 已签名的 `Runner_…_linux-….deb` 或 `.tar.gz` | glibc 2.35 或更高版本（Ubuntu 22.04 或更高版本），内核 5.3 或更高版本，ARM64 或 x86_64 |

在 macOS 上，Apple Silicon Mac 下载 `Runner-<version>-arm64.dmg`，Intel Mac 下载 `Runner-<version>-x64.dmg`；**苹果菜单 → 关于本机（Apple menu → About This Mac）** 会显示芯片。Intel 版本尚未在 Intel 硬件上验证。不支持 Windows ARM64。Linux 支持仍处于测试阶段：已在 Ubuntu 26.04 ARM64 虚拟机中试用，原生硬件、x86_64 和其他发行版尚未验证。在 Debian 或 Ubuntu 上，使用 `sudo apt install ./Runner_<version>_linux-<arm64|x64>.deb` 安装；在其他发行版上，解压 tarball 并运行其中的 `install.sh`。前置依赖、本地安装包与当前验证缺口见 `main` 上的 [Linux 构建与安装说明](https://github.com/yicheng47/runner/blob/main/docs/arch/linux.md)。

在 macOS 上，打开磁盘映像，将 Runner 拖入“应用程序”并启动。在 Windows 上，运行安装程序，再从开始菜单启动 Runner。新发布的 Windows 版本可能触发 SmartScreen 提示；确认下载的是官方安装程序后，选择 **更多信息 → 仍要运行（More info → Run anyway）** 继续安装。

Linux 本地安装包会注册应用菜单项和图标。从该菜单项启动 Runner；运行中的窗口使用相同桌面标识，使程序坞显示 Runner 名称和图标。

## 设置智能体

1. 按提供商说明安装智能体 CLI。[支持的智能体](../reference/supported-agents.md)列出了运行时与安装链接。默认结对编程小队的两个角色都使用 Codex。
2. 打开常用终端并运行智能体。完成登录或提供商配置，确认它能回答提示。
3. 打开 Runner 的 **设置 → 智能体（Settings → Agents）** 。确认智能体已启用，且显示可执行文件和版本。安装 CLI 后使用 **刷新（Refresh）** ；若可执行文件不在 `PATH` 中，使用 **浏览（Browse）** 选择它。

在 Windows 上，智能体原生运行，无需 WSL。Claude Code 和 pi 的 bash 工具需要 Git for Windows；通过 npm 安装的 CLI 还需要 Node.js。PowerShell 7 为可选项。

Runner 不会安装智能体 CLI，也不提供订阅。对支持更新的智能体，发现新版本时， **设置 → 智能体** 会提供 **更新（Update）** 。更新操作在终端中运行智能体自己的更新程序。

用量弹出面板中 **智能体设置（Agent settings）** 齿轮上的圆点表示某个智能体 CLI 有更新。打开用量面板或 **设置 → 智能体** 会刷新已安装版本；所有已启用智能体都更新后，圆点消失。更新 Runner 不会更新这些独立安装的 CLI。

## 启用 runner 命令

打开 **设置 → 通用 → 命令行（Settings → General → Command line）** 。第一次聊天不强制需要此命令，但从 Shell 工作或让另一个智能体操作 Runner 时很有用。

- **macOS：** 如果 `~/.local/bin` 或可写的 `/usr/local/bin` 已在登录 `PATH` 中，Runner 会自动安装链接。未安装时点击 **安装（Install）** 。设置页会显示目标路径及是否需要管理员密码。
- **Linux 本地安装包：** 安装程序会在打开 GUI 前提供 `runner`、`runnerd` 和 `runner-app`。tarball 安装需将 `~/.local/bin` 加入 PATH；系统包使用 `/usr/bin`。设置会将其识别为包管理的命令；请通过包安装程序移除。可选的 systemd 用户服务默认不启用。
- **Windows：** Runner 会将命令目录加入用户 `PATH`。请打开新终端以读取更改。

在该终端中检查命令：

```sh
runner --version
runner help
```

同一区域的 **供智能体使用的 Runner 技能（Runner skill for agents）** 开关会为检测到的智能体安装一份简短指南。它引导智能体读取与已安装 CLI 版本匹配的 `runner help agents`。

## 更新

Runner 原位更新，并保留设置、聊天和任务。有可用更新时，侧边栏 Runner 名称旁会出现一个图标。点击它即可下载更新，图标上的圆环显示下载进度。更新就绪后，点击图标会打开一个小弹窗，其中有 **重启以更新（Restart to update）** 、 **新功能（What's new）** 和 **稍后（Later）** ；如果下载是你发起的，弹窗会自动打开，并且不会移动键盘焦点。在下载、校验或安装期间，点击图标不会有任何反应，其工具提示会显示进度。如果下载失败或该更新只能手动下载，点击图标会打开 **设置 → 更新（Settings → Updates）** 。该页面在每个阶段都显示相同的状态，并提供 **取消（Cancel）** 、 **重试（Retry）** 、 **跳过此版本（Skip this version）** 和 **查看下载（View downloads）** 。在命令面板中， **显示 Runner 更新（Show Runner update）** 会在更新就绪时打开弹窗，其他情况下打开“设置 → 更新”，它不会开始下载。 **Runner → 检查更新（Check for Updates）** 会打开“设置 → 更新”并开始检查。

在 macOS 上，Runner 在后台检查，点击图标或“设置 → 更新”中的 **下载（Download）** 之前不会下载任何内容。每个 Mac 版本只会更新到相同架构；要在 Apple Silicon 和 Intel 版本之间切换，请手动安装另一个 DMG。仍在 Runner 0.6.0 或 0.6.1 的 Intel Mac 不会收到后续更新，需要手动安装一次 x64 DMG。在 Windows 和 Linux 上， **自动下载更新（Automatically download updates）** 默认开启；关闭后只通知你。 “设置 → 更新”中的 **跳过此版本（Skip this version）** 会隐藏该版本的图标，直到你下次手动检查。

**重启以更新（Restart to update）** 会关闭 Runner 并以新版本重新打开，不显示安装程序窗口。它会重启运行中的智能体，使其恢复已保存的对话；弹窗和“设置 → 更新”会在你选择前显示有多少智能体正在工作。 **稍后（Later）** 只关闭弹窗，图标会保留到更新安装完成。在 macOS 上，更新就绪时退出 Runner 也可能安装该更新。

Linux 构建使用正式版或 nightly 包标识，稳定版和 nightly 有各自的更新源。Linux 的 **设置 → 更新** 会下载并验证适合当前架构和安装格式的包。 **重启以更新（Restart to update）** 会原子替换受管理的用户 tar 安装，或请求系统授权通过 APT 安装 deb。系统安装需要 polkit（`pkexec`）。若 Runner 运行时外部安装程序或 APT 已替换可执行文件，请先重启 Runner 再检查或安装下一次更新；旧进程会显示重启错误并保持打开。取消授权或安装失败会保持 Runner 打开并显示错误； **设置 → 关于（Settings → About）** 列出安装日志。用户数据会保留。历史未签名版本只提供下载页，不会自动安装。通过 SSH 使用 CLI 前，请运行 `systemctl --user start runnerd` 显式启动 `runnerd`。[Linux 服务生命周期与 WSL 设置](https://github.com/yicheng47/runner/blob/main/docs/arch/linux.md#optional-systemd-user-service)介绍了持久运行和关闭行为。

Preview Nightly 构建是仍在独立分支上开发的功能的测试构建。它们面向 macOS、Windows 和 Linux 发布到单独的 [`preview-nightly` 预发布版本](https://github.com/yicheng47/runner/releases/tag/preview-nightly)，显示为 **Preview Nightly** 加上其短提交号，并且只从该版本更新。所有渠道使用同一应用和数据，因此切换时设置、聊天和任务都会保留。要离开 Preview Nightly，请手动安装 nightly 或稳定版构建；在 Linux 上，在 preview deb 之上安装同一版本的 nightly deb 会被 APT 视为降级。

接下来：[第一次聊天](first-chat.md)。
