<a id="example-llm-wiki"></a>

# LLM Wiki

Organize material from different sources into a Karpathy-style LLM Wiki of interlinked Markdown pages. Each page covers an entity, concept, or question, leading with a conclusion and citing its sources. An `index.md` page provides navigation.

The Skill picks the smallest page type that matches each page's retrieval purpose:

| Page type | Use for |
|-----------|---------|
| `entity` | A named thing with a stable identity (person, organization, product, project, system, dataset, standard, event…) |
| `concept` | A reusable idea, mechanism, pattern, protocol, or mental model |
| `method` | A reusable procedure with prerequisites, ordered steps, and a verifiable outcome |
| `comparison` | Two or more subjects evaluated side by side on explicit dimensions |
| `analysis` | A cross-source conclusion tied to a clear question |
| `summary` | A faithful digest of one source (only when `--instruction` explicitly asks for it) |

The default page types are `entity` and `concept`; other types follow the conditions in the Skill. Related material from multiple sources is grouped under the same topic.

Skill source: [openviking/system_assets/compile/example-skills/llm-wiki](https://github.com/volcengine/OpenViking/tree/main/openviking/system_assets/compile/example-skills/llm-wiki) · Visualization script: [examples/compile/graph-show/llm-wiki](https://github.com/volcengine/OpenViking/tree/main/examples/compile/graph-show/llm-wiki)

Check the [prerequisites](01-overview.md#prerequisites) and run these commands from the OpenViking repository root. Replace the source directory with your own material.

<a id="step-1-prepare-the-sources"></a>

## Prepare the sources

If the material is not in OpenViking yet, import it. Use `ov add-resource` for directories, `ov write` for a single file:

```bash
# Import a directory as a source
ov add-resource ./my-research --to viking://resources/research --wait

# Or write a single file
ov mkdir viking://resources/research
ov write viking://resources/research/notes.md \
  --from-file ./notes.md --mode create
```

Confirm the source is in place:

```bash
ov ls -r viking://resources/research
```

<a id="step-2-add-the-skill"></a>

## Add the Skill

This example installs the Skill under `viking://agent/skills`, matching the compile command below. Omit `-p` for a private installation, then use the returned URI in `--skill`:

```bash
ov add-skill openviking/system_assets/compile/example-skills/llm-wiki -p viking://agent/skills --wait
```

Find the installed Skill URI:

```bash
ov skills list
# → viking://agent/skills/llm-wiki
```

<a id="step-3-run-compile"></a>

## Run compile

```bash
ov compile \
  --from viking://resources/research \
  --to viking://resources/research-wiki \
  --skill viking://agent/skills/llm-wiki \
  --instruction "Organize into a team-searchable Wiki, keeping the source of every claim"
```

- `--from` can be repeated or comma-separated to pass multiple sources at once.
- The `--to` directory is created automatically if it does not exist.
- Add `-o json` for machine-readable output. The command returns a `task_id` immediately; use it to inspect or cancel the task:

```bash
ov task status cmp_01abc      # progress and final result
ov task cancel cmp_01abc      # cooperative cancel
```

<a id="step-4-inspect-the-output"></a>

## Inspect the output

When compile finishes, the target directory holds a Markdown knowledge base. Read the navigation page first, then drill in:

```bash
ov tree viking://resources/research-wiki
ov read viking://resources/research-wiki/index.md
```

Typical layout (page type maps to directory):

```text
research-wiki/
├── index.md            # navigation entry, type index
├── entity/
│   └── <title>.md
├── concept/
│   └── <title>.md
├── method/…  comparison/…  analysis/…
```

<a id="step-5-visualize-it-as-an-interactive-graph"></a>

## Visualize it as an interactive graph

`wiki_graph.py` connects **directly to the OpenViking service** to read the Wiki pages (no local download needed), colors pages by type, links them by their cross-references, and produces a standalone interactive HTML:

```bash
python examples/compile/graph-show/llm-wiki/wiki_graph.py \
  viking://resources/research-wiki \
  -o research-wiki-graph.html \
  --title "Research Knowledge Base"
```

Open `research-wiki-graph.html` in a browser. Nodes are pages (colored by `entity`/`concept`/`method`…), edges are links between pages, and clicking a node shows its body.

Connection settings resolve the same way as `ov`: command-line arguments → `OPENVIKING_*` environment variables → `~/.openviking/ovcli.conf`. Pass them explicitly for a remote service:

```bash
python examples/compile/graph-show/llm-wiki/wiki_graph.py \
  viking://resources/research-wiki \
  --url https://openviking.example.com \
  --api-key "$OPENVIKING_API_KEY" \
  -o research-wiki-graph.html --title "Research Knowledge Base"
```

Pass multiple Wikis to draw them on the same graph for comparison:

```bash
python examples/compile/graph-show/llm-wiki/wiki_graph.py \
  viking://resources/wiki-a viking://resources/wiki-b \
  -o combined.html --title "Two Knowledge Bases Side by Side"
```

## Sample page

This excerpt comes from the [Lighthouse sample page](/quickstart/wiki-output-en-v1.txt), using the onboarding tutorial data. It illustrates Wiki content rather than a fixed result for the `research` directory above. Your sources determine page content, names, and counts.

```markdown
---
type: entity
title: Lighthouse
description: The fictional v1 pilot project running on an isolated network without internet access, including its offline installation, local feedback workflow, and undecided items.
tags: [lighthouse, pilot, isolated-network]
---

# Lighthouse

Lighthouse is a fictional tutorial sample (version 1) that runs as a pilot on an isolated network without internet access. It is the central project of this Wiki: its offline setup, weekly feedback loop, and preparation steps are documented below, while several dates and scope items remain unknown or undecided in the sources.

## Identity and setup

- Fictional tutorial sample v1; the first version runs in a pilot on an isolated network without internet access ([Lighthouse project](viking://resources/onboarding-en-v1/project/project.md)).
- The pilot administrator prepares an offline installation package and a local storage directory ([Lighthouse project](viking://resources/onboarding-en-v1/project/project.md)).

## Feedback workflow

```

Open the source URIs and check key facts. Clicking that page in the graph should show its corresponding text and links.

## Related docs

- [Context compilation](./01-overview.md)
- [Knowledge Graph example](./03-knowledge-graph.md)
- [Agent Runtime API](../api/23-agent-runtime.md)
