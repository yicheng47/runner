# 组建小队

创建可复用角色，将其组成小队，并明确成员之间的交接。第一个团队只需一个 coder 和一个 reviewer。

## 创建角色

打开 **角色（Roles）** 并创建两个角色。每个角色选择已安装并登录的智能体。可以都使用同一运行时，也可以混用提供商；每个都需要可正常工作的安装。

| 角色 | 建议指令 |
| --- | --- |
| Coder，代号 `coder` | 实现请求的修改，遵循本地模式，运行相关检查并修复审查发现的问题。严格遵守目标范围。 |
| Reviewer，代号 `reviewer` | 阅读简报与工作区差异。用文件和行号报告必须修复的缺陷，或明确说明审查通过。不要编辑文件。 |

若代号已存在，请使用已保存角色或选择其他代号。角色提示词不要包含具体任务要求，以便在直接聊天中也可复用。

## 组合小队

1. 打开 **小队（Crews）** ，创建小队并命名。
2. 添加 coder 和 reviewer 角色，为槽位指定不同代号。
3. 检查每个槽位的运行时、模型与推理强度。除非需要覆盖，否则保留继承值。若更改运行时，请重新检查该运行时的模型和推理强度。
4. 添加团队约定，例如：

   ```text
   The coder implements; the reviewer waits for a handoff. Before editing, the coder checks repository instructions and existing changes. The coder sends the reviewer a summary, changed files and checks run. The reviewer checks the working-tree diff without editing. Repeat until the review is clean. Leave all changes uncommitted unless the mission goal explicitly authorizes commits or a pull request. Ask the person if a decision changes scope.
   ```

团队约定适用于任务中的每个成员。角色提示词描述职责；约定描述这些职责如何衔接。

## 为小队设定目标

启动任务时说明具体成果、工作目录、应运行的检查，以及是否允许提交、推送或发布。例如：“为报告的解析错误添加回归测试，修复错误，并由 reviewer 验证两者。保留未提交的修改。”

Runner 不会自动隔离工作目录。如需在 Git worktree 中工作，请先创建它，再选择其路径。没有明确负责人时，不要让多个执行者重叠编辑同一内容。

## 使用示例

### 结对编程起步小队

全新安装不再在尚未安装任何智能体前预置小队。首次 **设置引导** 会在检测到可用智能体时提议创建 **Pair coding**，这是一个双角色审查流程，并尽可能为两个槽位分配不同的可用运行时（优先默认与已登录的智能体），或在一个可用时共用同一运行时。随时可在 **小队（Crews）** 中切换任一槽位的智能体，或在尚无小队时从 **设置 → 智能体** 点击 **打开引导**。

| 角色 | 职责 | 系统提示词 |
| --- | --- | --- |
| **@coder** | 创建任务分支，实现修改，运行检查，将差异交给 reviewer，并修复问题。 | [coder.md](https://github.com/yicheng47/runner/blob/main/examples/pair-coding/coder.md) |
| **@reviewer** | 检查工作区差异，以文件与行号报告必须修复的问题，不编辑代码。 | [reviewer.md](https://github.com/yicheng47/runner/blob/main/examples/pair-coding/reviewer.md) |

[团队约定](https://github.com/yicheng47/runner/blob/main/examples/pair-coding/team-conventions.md)要求在任务分支工作，并在提交前通过审查。除非任务目标明确授权 Git 操作，否则修改保持未提交；合并始终需要操作者指令。

### 更多小队

仓库提供可直接复制的提示词与约定：

- [Pair coding](https://github.com/yicheng47/runner/tree/main/examples/pair-coding)：预置的 coder/reviewer 流程。
- [Codex trio](https://github.com/yicheng47/runner/tree/main/examples/codex-trio)：coder、reviewer 和 QA；QA 构建已审查版本，并在运行的应用中验证。
- [Development crew](https://github.com/yicheng47/runner/tree/main/examples/dev-crew)：架构师、实现者和审查者。
- [Documentation crew](https://github.com/yicheng47/runner/tree/main/examples/docs-crew)：架构师、作者与编辑。
- [Tic-tac-toe](https://github.com/yicheng47/runner/tree/main/examples/tic-tac-toe)：两位玩家和一位裁判。
- [Werewolf](https://github.com/yicheng47/runner/tree/main/examples/werewolf)：六位玩家和一位主持人。
- [Tomb raid](https://github.com/yicheng47/runner/tree/main/examples/tomb-raid)：由游戏主持人带领的四人夺宝小队。

将角色提示词复制到角色中，将约定复制到小队中。示例目标只是起点，请替换为自己的任务。以上链接显示 `main` 上的当前示例；旧版本示例请使用仓库的标签选择器。

[第一个任务](../getting-started/first-mission.md)介绍如何启动并跟进团队。
