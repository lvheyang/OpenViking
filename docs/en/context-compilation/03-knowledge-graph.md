<a id="example-knowledge-graph"></a>

# Knowledge graph

Extract typed entities and typed, directed relationships into a knowledge graph, keeping source references for each statement and edge. The output contains entity files and an edge list:

```text
entities/
  <entity-id>.md      # one file per node; frontmatter carries type/id/title/entity_type/description/sources
relations.jsonl       # one directed edge per line
```

Each edge is a compact JSON line, readable as the statement `<from> <relation> <to>`:

```json
{"from":"sun-wukong","relation":"member_of","label":"belongs to","to":"pilgrimage-team","evidence":["viking://resources/source.md"]}
```

`relation` is a stable, language-independent machine predicate (`member_of`, `leads`, `located_in`…), `label` is its localized display name, and `entity_type` drives node color, shape, and filtering in the visualization. The graph refreshes incrementally: existing nodes and edges are preserved, evidence is merged, and new knowledge is appended.

Skill source: [examples/compile/ov-compile-skills/knowledge-graph](https://github.com/volcengine/OpenViking/tree/main/examples/compile/ov-compile-skills/knowledge-graph) · Visualization script: [examples/compile/graph-show/knowledge-graph](https://github.com/volcengine/OpenViking/tree/main/examples/compile/graph-show/knowledge-graph)

Check the [prerequisites](01-overview.md#prerequisites) and run these commands from the OpenViking repository root. Replace the source directory with your own material.

<a id="step-1-prepare-the-sources"></a>

## Prepare the sources

```bash
ov add-resource ./journal-to-the-west --to viking://resources/journal --wait
ov ls -r viking://resources/journal
```

<a id="step-2-add-the-skill"></a>

## Add the Skill

```bash
ov add-skill examples/compile/ov-compile-skills/knowledge-graph -p viking://agent/skills --wait
ov skills list
# → viking://agent/skills/knowledge-graph
```

<a id="step-3-run-compile"></a>

## Run compile

```bash
ov compile \
  --from viking://resources/journal \
  --to viking://resources/journal-kg \
  --skill viking://agent/skills/knowledge-graph \
  --instruction "Extract characters, places, artifacts and their relationships into a traversable graph"
```

The command returns a `task_id` immediately. Then:

```bash
ov task status cmp_01abc      # progress and final result
ov task cancel cmp_01abc      # cooperative cancel
```

<a id="step-4-inspect-the-output"></a>

## Inspect the output

```bash
ov tree viking://resources/journal-kg
ov read viking://resources/journal-kg/relations.jsonl
ov read viking://resources/journal-kg/entities/sun-wukong.md
```

<a id="step-5-visualize-it-as-an-interactive-graph"></a>

## Visualize it as an interactive graph

Unlike the LLM Wiki script, `knowledge_graph.py` reads from a **local directory** (it needs both `entities/` and `relations.jsonl` on disk). So download the output first, then generate the HTML.

`ov get` downloads one file at a time. This script downloads only the entity Markdown files and edge list:

```bash
SRC="viking://resources/journal-kg"
DST="./journal-kg"
mkdir -p "$DST"
ov ls -r -s "$SRC" | while read -r uri; do
  # only download files (entities/*.md and relations.jsonl), skip directories
  case "$uri" in
    "$SRC"/entities/*.md|"$SRC"/relations.jsonl) ;;
    *) continue ;;
  esac
  rel="${uri#$SRC/}"
  mkdir -p "$DST/$(dirname "$rel")"
  ov get "$uri" "$DST/$rel"
done
```

> `ov get` requires the local target file to not exist. To download again, change `DST` to a new directory and keep the previous result.

Confirm the local layout is correct:

```bash
find ./journal-kg          # should show entities/*.md and relations.jsonl
```

Generate the interactive HTML:

```bash
python examples/compile/graph-show/knowledge-graph/knowledge_graph.py \
  ./journal-kg \
  -o journal-kg.html \
  --title "Journey to the West Knowledge Graph"
```

Open `journal-kg.html` in a browser. The script validates first — `relations.jsonl` must be valid JSON, every entity file must have a stable `id` and `title`, and both ends of every edge must resolve to an entity node — and fails with the offending line if not, so it doubles as a quality check on the output. Nodes are colored and shaped by `entity_type`, edges show their localized `label`, and clicking a node reveals that entity's body, aliases, and sources.

## Verify the artifact

Illustrative structure, not captured output:

```text
entities/
  sun-wukong.md
relations.jsonl
```

An entity page should describe Sun Wukong and cite sources; `from` and `to` in the relation table must resolve to entity IDs. The introductory `member_of` JSON illustrates the structure, not a captured run. After visualization, click an entity to read its text and verify evidence for the membership edge.

## Related docs

- [Context compilation](./01-overview.md)
- [LLM Wiki example](./02-llm-wiki.md)
- [Agent Runtime API](../api/23-agent-runtime.md)
