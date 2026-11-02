#!/usr/bin/env python3
"""Re-score retained archives without network requests or rewriting source data."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from urllib.parse import unquote

from reference_urls import first_hit_rank

ROOT = Path(__file__).resolve().parent.parent
VERSION = re.compile(r"^(?:stable|current|latest|v?\d+(?:\.\d+)*|\d+(?:\.\d+)*)$")


def legacy_rank(urls: list[str], gold: list[str]) -> int | None:
    def normalise(value: str) -> str:
        rest = unquote(value.split("://", 1)[-1]).rstrip("/").lower()
        host, _, path = rest.partition("/")
        return host + "/" + "/".join("*" if VERSION.match(part) else part for part in path.split("/"))
    return next((i for i, url in enumerate(urls, 1) if any(normalise(url).startswith(normalise(ref)) for ref in gold)), None)


def summary(ranks: list[int | None]) -> dict:
    n = len(ranks)
    return {"questions": n, "hit@1": sum(r == 1 for r in ranks),
            "hit@5": sum(r is not None and r <= 5 for r in ranks),
            "hit@10": sum(r is not None and r <= 10 for r in ranks),
            "mrr@10": round(sum(1 / r for r in ranks if r is not None and r <= 10) / n, 6) if n else 0}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    gold_path = ROOT / "bench/quality-questions.json"
    golds = {row["id"]: row["gold"] for row in json.loads(gold_path.read_text(encoding="utf-8"))["queries"]}
    archives = []
    for path in sorted((ROOT / "bench").glob("*.json")):
        if not (path.name.startswith("eval-100-") or path.name.startswith("content-")):
            continue
        data = json.loads(path.read_text(encoding="utf-8"))
        rows = data.get("rows", data.get("queries", []))
        old, corrected, changes = [], [], []
        for row in rows:
            gold = row.get("gold") or golds.get(row["id"]) or [row["gold_url"]]
            if isinstance(gold, str):
                gold = [gold]
            urls = (row.get("urls") or [result.get("url", "") for result in row.get("results", [])])[:10]
            before = legacy_rank(urls, gold)
            after = first_hit_rank(urls, gold)
            old.append(before)
            corrected.append(after)
            if before != after:
                changes.append({"id": row["id"], "old_rank": before, "corrected_rank": after})
        archives.append({"source": path.relative_to(ROOT).as_posix(), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                         "legacy_recomputed": summary(old), "corrected": summary(corrected), "changed_rows": changes})
    report = {"format_version": 1, "metric": "legacy-version-tolerant-structured-url-prefix-v2",
              "matcher_sha256": hashlib.sha256((ROOT / "tools/reference_urls.py").read_bytes()).hexdigest(),
              "gold_file": gold_path.relative_to(ROOT).as_posix(), "gold_sha256": hashlib.sha256(gold_path.read_bytes()).hexdigest(),
              "limitations": ["No new requests; these are historical observations, not current defaults.",
                              "Numeric/stable/current/latest path segments retain the legacy version-alias rule.",
                              "No retained Exa or Tavily per-question content archive is present; their published scores were not recomputed."],
              "archives": archives}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    for item in archives:
        print(item["source"], item["legacy_recomputed"]["hit@1"], "->", item["corrected"]["hit@1"], "changed rows", len(item["changed_rows"]))


if __name__ == "__main__":
    main()
