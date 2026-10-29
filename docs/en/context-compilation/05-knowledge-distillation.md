<a id="example-knowledge-distillation"></a>

# Knowledge distillation

Extract conclusions from one or more knowledge bases, organize them by topic, and retain source references and uncertainties.

For example, compare financial reports to analyze changes in revenue, profitability, and risk over time.

The output uses one directory per topic and one page per conclusion:

```text
revenue-quality/
  growth-shifted-from-volume-to-pricing.md
  overseas-growth-offset-domestic-slowdown.md
profitability/
  margin-recovered-but-cash-conversion-weakened.md
risk/
  customer-concentration-increased.md
```

> This is a shape example only — the real topics and findings come from the domain you supply. Note that page names state the **conclusion itself** (`growth-shifted-from-volume-to-pricing`), not a source title (`q2-report-summary`).

Skill source: [openviking/system_assets/compile/example-skills/knowledge-distillation](https://github.com/volcengine/OpenViking/tree/main/openviking/system_assets/compile/example-skills/knowledge-distillation)

Check the [prerequisites](01-overview.md#prerequisites) and run these commands from the OpenViking repository root. Replace the source directory with your own material.

<a id="step-1-prepare-the-sources"></a>

## Prepare the sources

```bash
ov add-resource ./finance-reports --to viking://resources/finance-reports --wait
ov ls -r viking://resources/finance-reports
```

<a id="step-2-add-the-skill"></a>

## Add the Skill

```bash
ov add-skill openviking/system_assets/compile/example-skills/knowledge-distillation -p viking://agent/skills --wait
ov skills list
# → viking://agent/skills/knowledge-distillation
```

<a id="step-3-run-compile"></a>

## Run compile

Spell out the **analytical question, comparison dimensions, baseline, and scope** in `--instruction` — it directly sets the direction of the distillation:

```bash
ov compile \
  --from viking://resources/finance-reports \
  --to viking://resources/finance-insights \
  --skill viking://agent/skills/knowledge-distillation \
  --instruction "Compare the last three years of reports; obtain changes and drivers in revenue quality, profitability, and risk"
```

`--from` accepts multiple sources for cross-knowledge-base comparison:

```bash
ov compile \
  --from viking://resources/finance-2024,viking://resources/finance-2025 \
  --to viking://resources/finance-insights \
  --skill viking://agent/skills/knowledge-distillation \
  --instruction "Compare the two yearly knowledge bases; surface changes and structural differences in key metrics"
```

The command returns a `task_id` immediately:

```bash
ov task status cmp_01abc      # progress and final result
ov task cancel cmp_01abc      # cooperative cancel
```

<a id="step-4-inspect-the-output"></a>

## Inspect the output

List the output first, then read a generated conclusion page. The filename below is illustrative:

```bash
ov tree viking://resources/finance-insights
ov read viking://resources/finance-insights/revenue-quality/growth-shifted-from-volume-to-pricing.md
```

By default **no `index.md` is created** — a distillation is itself a set of conclusions, unless `--instruction` explicitly asks for a navigation page. Re-running refreshes the same analysis page and time-bounds any conclusion that may change.

## Verify the artifact

Illustrative structure, not captured output:

```markdown
# Revenue growth drivers changed
## Conclusion and evidence
Compare revenue components and cite evidence for volume and pricing changes.
## Uncertainty
Separate reported figures from inference; retain unknowns where definitions differ.
```

This illustrates an analysis page, not a financial finding. Actual pages should specify years, metric definitions, sources, and uncertainty. Read the cited materials to check the conclusion and ensure inferences are labeled.

## Related docs

- [Context compilation](./01-overview.md)
- [Daily Report example](./04-daily-report.md)
- [Agent Runtime API](../api/23-agent-runtime.md)
