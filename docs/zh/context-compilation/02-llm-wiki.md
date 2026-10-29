<a id="示例-llm-wiki"></a>

# LLM Wiki

将不同来源的材料整理成 Karpathy 风格的 LLM Wiki，即互相链接的 Markdown 知识库。每页围绕一个实体、概念或问题，先给结论，再列出处；`index.md` 提供导航。

这套 Skill 会按页面的检索目的挑选最合适的页面类型：

| 页面类型 | 用于 |
|---------|------|
| `entity` | 有稳定身份的具名事物（人、组织、产品、项目、系统、数据集、标准、事件……） |
| `concept` | 可复用的思想、机制、模式、协议、心智模型 |
| `method` | 有前置条件、有序步骤、可验证结果的可复用流程 |
| `comparison` | 在明确维度上对两个及以上对象做并排评估 |
| `analysis` | 围绕一个问题的跨来源结论 |
| `summary` | 单一来源的忠实数字化摘要（仅当 `--instruction` 明确要求时才生成） |

默认生成 `entity` 和 `concept` 页面，其他类型按 Skill 中的条件生成。多个来源中的相关内容会合并到同一主题下。

Skill 源码：[openviking/system_assets/compile/example-skills/llm-wiki](https://github.com/volcengine/OpenViking/tree/main/openviking/system_assets/compile/example-skills/llm-wiki) · 可视化脚本：[examples/compile/graph-show/llm-wiki](https://github.com/volcengine/OpenViking/tree/main/examples/compile/graph-show/llm-wiki)

先确认[前置条件](01-overview.md#前置条件)，并在 OpenViking 仓库根目录运行以下命令。来源目录需替换为自己的资料目录。

<a id="第一步-准备来源"></a>

## 准备来源

如果材料还没进 OpenViking，先导入。目录型来源用 `ov add-resource`，单文件可以用 `ov write`：

```bash
# 导入一个目录作为来源
ov add-resource ./my-research --to viking://resources/research --wait

# 或者写入单个文件
ov mkdir viking://resources/research
ov write viking://resources/research/notes.md \
  --from-file ./notes.md --mode create
```

确认来源已就位：

```bash
ov ls -r viking://resources/research
```

<a id="第二步-添加-skill"></a>

## 添加 Skill

本例将 Skill 安装到共享目录 `viking://agent/skills`，与后面的编译命令保持一致。省略 `-p` 会安装到用户私有目录，此时应把 `--skill` 改为安装返回的 URI：

```bash
ov add-skill openviking/system_assets/compile/example-skills/llm-wiki -p viking://agent/skills --wait
```

查看装好的 Skill URI：

```bash
ov skills list
# → viking://agent/skills/llm-wiki
```

<a id="第三步-执行编译"></a>

## 执行编译

```bash
ov compile \
  --from viking://resources/research \
  --to viking://resources/research-wiki \
  --skill viking://agent/skills/llm-wiki \
  --instruction "面向团队检索整理成 Wiki，保留每条结论的出处"
```

- `--from` 可以重复或用逗号分隔，一次传多个来源。
- `--to` 目录不存在时会自动创建。
- 想要机器可读结果加 `-o json`；命令会立即返回 `task_id`，用它查询或取消任务：

```bash
ov task status cmp_01abc      # 查看进度与最终结果
ov task cancel cmp_01abc      # 协作式取消
```

<a id="第四步-查看产物"></a>

## 查看产物

编译完成后目标目录里就是一套 Markdown 知识库。先看导航页，再按需钻进去：

```bash
ov tree viking://resources/research-wiki
ov read viking://resources/research-wiki/index.md
```

典型结构（页面类型对应目录）：

```text
research-wiki/
├── index.md            # 导航入口，类型 index
├── entity/
│   └── <标题>.md
├── concept/
│   └── <标题>.md
├── method/…  comparison/…  analysis/…
```

<a id="第五步-可视化成交互式图谱"></a>

## 可视化成交互式图谱

`wiki_graph.py` 会**直接连接 OpenViking 服务**读取 Wiki 页面（不需要先下载到本地），把页面按类型着色、按链接连边，生成一个独立的交互式 HTML：

```bash
python examples/compile/graph-show/llm-wiki/wiki_graph.py \
  viking://resources/research-wiki \
  -o research-wiki-graph.html \
  --title "研究知识库"
```

用浏览器打开 `research-wiki-graph.html` 即可。节点是页面（按 `entity`/`concept`/`method`… 分色），边是页面之间的链接，点节点能看正文。

连接配置的解析顺序和 `ov` 一致：命令行参数 → `OPENVIKING_*` 环境变量 → `~/.openviking/ovcli.conf`。远程服务显式传参：

```bash
python examples/compile/graph-show/llm-wiki/wiki_graph.py \
  viking://resources/research-wiki \
  --url https://openviking.example.com \
  --api-key "$OPENVIKING_API_KEY" \
  -o research-wiki-graph.html --title "研究知识库"
```

一次传多个 Wiki，可以把它们画在同一张图里对比：

```bash
python examples/compile/graph-show/llm-wiki/wiki_graph.py \
  viking://resources/wiki-a viking://resources/wiki-b \
  -o combined.html --title "两个知识库对照"
```

## 正文示例

下面摘自[灯塔项目示例正文](/quickstart/wiki-output-zh-v1.txt)，对应入门教程的固定资料；它用于展示 Wiki 正文，不是上方 `research` 目录的预设输出。自己的页面内容、名称和数量由来源决定。

```markdown
---
type: entity
title: 灯塔项目
description: 灯塔项目第一版是在隔离网络中运行的反馈试点，反馈以本地文件形式每周五离线导出。
tags: [灯塔项目, 隔离网络, 反馈试点, v1]
---

# 灯塔项目

灯塔项目是一个虚构教学样例（v1），其第一版用于隔离网络中的反馈试点，运行时无法访问外网。试点的核心机制是：用户反馈保存为本地文件，由试点管理员每周五导出并交给项目负责人；第一版不支持云端同步，是否在后续版本提供尚未决定。

## 试点准备

- 试点管理员需提前准备离线安装包和本地存储目录，以保证在无法访问外网的环境中运行。[灯塔项目](viking://resources/onboarding-zh-v1/project/project.md)
- 试点开始前要准备反馈登记模板，并安排管理员演练一次离线导出（项目负责人在试点准备会议上确认）。[试点准备会议](viking://resources/onboarding-zh-v1/meeting/meeting.md)

## 反馈流程

```

打开正文中的来源 URI，核对关键事实；图谱中点击同一页面，应看到对应正文与链接。

## 相关文档

- [上下文编译](./01-overview.md)
- [Knowledge Graph 示例](./03-knowledge-graph.md)
- [Agent Runtime API](../api/23-agent-runtime.md)
