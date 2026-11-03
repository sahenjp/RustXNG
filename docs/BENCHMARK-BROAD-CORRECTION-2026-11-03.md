# Broad/fresh target-domain scoring correction — 2026-11-03

PR [#81](https://github.com/sahenjp/RustXNG/pull/81) attaches each case's target domains before calculating its metrics.
Previously the observation retained the targets, but the scorer saw an empty list and stored zero coverage.

The [additive correction record](../bench/rescore-broad-2026-11-03.json) records the original observation-file SHA-256,
the evaluator commit, all 24 affected rows and the corrected aggregates. Original observations and summaries are retained.

| Language condition | Observations | Stored mean coverage | Corrected mean coverage |
| --- | --- | --- | --- |
| default | 12 | 0.0000 | 0.3889 |
| ja | 12 | 0.0000 | 0.3889 |

This is a correction to the evaluator, not a new search-quality measurement or a claim of improved retrieval.
No provider search or LLM call was made. Only target-domain coverage and seen-domain lists were recomputed.
Freshness metrics were retained: the old evaluator uses the current time for its cutoff, so re-running those fields later would change the evaluation window.

Reproduce the coverage aggregates from the retained observations on the corrected evaluator:

```bash
python3 - <<'PYCODE'
import gzip, json, statistics, sys
sys.path.insert(0, "tools")
from eval_japanese import broad_metrics
with gzip.open("bench/results/japanese-2026-11-02/observations.jsonl.gz", "rt", encoding="utf-8") as stream:
    rows = [json.loads(line) for line in stream]
for language in ("default", "ja"):
    selected = [row for row in rows if row["group"] == "broad_fresh" and row["language_condition"] == language]
    values = [broad_metrics(row, 30)["target_source_domain_coverage"] for row in selected]
    print(language, len(values), statistics.mean(values))
PYCODE
```
