<a id="context-compilation-overview"></a>

# Context compilation

`ov compile` reads documents, notes, web pages, or session records from OpenViking and uses a Skill to organize them into a Wiki, knowledge graph, daily report, or other output stored in OpenViking.

## How it works

Compilation with an installed Skill needs:

- `--from`: one or more source directories or files.
- `--to`: the output directory.
- `--skill`: the URI of an installed Skill that defines the output's content and structure.

Use the optional `--instruction` to specify this run's scope, audience, language, emphasis, or date range.

The configured [Agent Runtime](../api/23-agent-runtime.md) runs the compilation; local deployments can use the built-in [VikingBot](../concepts/15-vikingbot.md). It reads the sources and Skill under the requesting user's identity, then organizes and writes content in a dedicated agent loop. The task runs asynchronously and returns a `task_id` for checking progress and results.

## Prerequisites

- For the Skill-based examples, a running OpenViking service with a Compile Runtime configured. For local use, install the `openviking[bot]` dependencies and enable the built-in VikingBot with `--with-bot`. The `--skill memory` mode runs in-process without a Compile Runtime and takes no `--from`. The default endpoint is `http://localhost:1933`; remote use needs an API Key — see [Authentication](../guides/04-authentication.md). No service yet? Start with the [Quick Start](../getting-started/02-quickstart.md).
- The `ov` CLI configured with a connection (`~/.openviking/ovcli.conf`, or a file selected by `OPENVIKING_CLI_CONFIG_FILE`).
- Paths beginning with `examples/...` are relative to the repository. Download the [OpenViking repository](https://github.com/volcengine/OpenViking) and run the commands from its root.
- Python 3 for the visualization scripts. The [LLM Wiki script](https://github.com/volcengine/OpenViking/blob/main/examples/compile/graph-show/llm-wiki/wiki_graph.py) also requires the `openviking` Python package.

## Run it in one command

First import the sources and install the Skill using the [LLM Wiki example](02-llm-wiki.md), then run:

```bash
ov compile \
  --from viking://resources/research \
  --to viking://resources/research-wiki \
  --skill viking://agent/skills/llm-wiki \
  --instruction "Organize the research into a knowledge base the team can search"
```

The command returns a `cmp_...` task ID immediately. Use `ov task status <id>` to check progress and `ov task cancel <id>` to stop it. The full field reference, task lifecycle, and HTTP API are in the [Agent Runtime API](../api/23-agent-runtime.md).

## Check the output

Once the task completes, open the output directory's index and read a topic page to check its sources:

```bash
ov tree viking://resources/research-wiki
ov read viking://resources/research-wiki/index.md
```

Wiki compilation organizes sources into topic pages and an index. Read the [Lighthouse sample page](/quickstart/wiki-output-en-v1.txt) to see how conclusions and citations are organized, then use [LLM Wiki](02-llm-wiki.md) to inspect your own output. Task success describes processing; factual content and sources still need checking.

## Use Web Studio

Studio provides a Compile list at `/compile`, a form at `/compile/new`, and task details at `/compile/tasks/<task_id>`. Select and preview a Skill, browse source directories, submit multiple sources, inspect or cancel a task, and open its output directory. The Filesystem terminal also supports `compile` and `task`; it can hand parameters to the form.

For submission retries, pagination, and record retention, see [Agent Runtime API](../api/23-agent-runtime.md#compile-request-lifecycle).

## Swap the Skill, get a different output

The Skill defines the output. The repository includes these examples; LLM Wiki and Knowledge Graph also include visualization scripts:

| Skill | Output shape | Good for | Example |
|-------|-------------|----------|---------|
| **LLM Wiki** | A set of interlinked Markdown pages (entity, concept, method…) plus a navigation `index.md` | A knowledge base that both people and agents can quickly search, navigate, and reuse | [LLM Wiki example](./02-llm-wiki.md) |
| **Knowledge Graph** | `entities/*.md` nodes plus a `relations.jsonl` edge file | Structured knowledge you traverse by entity, type, and relationship | [Knowledge Graph example](./03-knowledge-graph.md) |
| **Daily Report** | One `<YYYY-MM-DD>.md` page per date | Reconstructing "what actually happened each day" from conversations, sessions, messages, and task records | [Daily Report example](./04-daily-report.md) |
| **Knowledge Distillation** | Topic-organized high-level conclusion pages | Distilling cross-source findings, trends, and changes out of one or more knowledge bases | [Knowledge Distillation example](./05-knowledge-distillation.md) |

The first two examples also give the complete `ov` commands from **importing sources → adding the Skill → running compile → visualizing the output**, ending in an interactive HTML graph.

## Not only new artifacts: consolidating existing memories

Besides compiling source material into new knowledge artifacts with a Skill, `ov compile` has a special mode — set `--skill` to `memory` to consolidate an existing **memory** directory in place (dedup, merge, split, compact), without going through VikingBot. See [Memory Consolidation](./06-memory-consolidation.md).


## Related docs

- [VikingBot concepts](../concepts/15-vikingbot.md) — the built-in Compile runtime
- [Agent Runtime API](../api/23-agent-runtime.md) — full reference for creating, inspecting, and cancelling Compile tasks
- [Skills API](../api/04-skills.md) — managing and customizing Skills
