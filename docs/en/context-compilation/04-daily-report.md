<a id="example-daily-report"></a>

# Daily report

Compile timestamped conversation logs, agent sessions, IM messages, collaborative documents, meeting notes, and task records into concise, evidence-grounded **daily reports**: one page per day.

Skill source: [openviking/system_assets/compile/example-skills/daily-report](https://github.com/volcengine/OpenViking/tree/main/openviking/system_assets/compile/example-skills/daily-report)

Check the [prerequisites](01-overview.md#prerequisites) and run these commands from the OpenViking repository root. Replace the source directory with your own material.

<a id="step-1-prepare-the-sources"></a>

## Prepare the sources

Daily-report sources are usually sessions, messages, or documents already in OpenViking. To import a batch of records from local:

```bash
ov add-resource ./work-logs --to viking://resources/work-logs --wait
ov ls -r viking://resources/work-logs
```

<a id="step-2-add-the-skill"></a>

## Add the Skill

```bash
ov add-skill openviking/system_assets/compile/example-skills/daily-report -p viking://agent/skills --wait
ov skills list
# → viking://agent/skills/daily-report
```

<a id="step-3-run-compile"></a>

## Run compile

Spell out the **date, timezone, report subject, and emphasis** in `--instruction` — the Skill uses it to scope and prioritize:

```bash
ov compile \
  --from viking://resources/work-logs \
  --to viking://resources/daily-report \
  --skill viking://agent/skills/daily-report \
  --instruction "Daily report for 2026-08-20 in Asia/Shanghai, focused on my outcomes and decisions"
```

For several days at once, put the date range in `--instruction` (each day is still its own page):

```bash
ov compile \
  --from viking://resources/work-logs \
  --to viking://resources/daily-report \
  --skill viking://agent/skills/daily-report \
  --instruction "One daily report per day for 2026-08-18 to 2026-08-20 in Asia/Shanghai"
```

The command returns a `task_id` immediately:

```bash
ov task status cmp_01abc      # progress and final result
ov task cancel cmp_01abc      # cooperative cancel
```

<a id="step-4-inspect-the-output"></a>

## Inspect the output

Reports are plain Markdown — just read them:

```bash
ov tree viking://resources/daily-report
ov read viking://resources/daily-report/2026-08-20.md
```


## Verify the artifact

Illustrative structure, not captured output:

```markdown
# Daily report: 2026-08-20
## Completed
- Practiced feedback export. [Source](viking://resources/work-logs/demo.md)
## Open items
- The final delivery date for the offline installer remains unconfirmed.
```

This fictional example illustrates the page structure; actual output must follow the source records. Check dates, time zone, completion state, owners, and citations. Pending work must remain pending, and undocumented events must not be invented.

## Related docs

- [Context compilation](./01-overview.md)
- [Knowledge Distillation example](./05-knowledge-distillation.md)
- [Agent Runtime API](../api/23-agent-runtime.md)
