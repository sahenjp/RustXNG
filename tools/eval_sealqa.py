#!/usr/bin/env python3
"""Measure SealQA reference-page retrieval, not official QA answer accuracy.

Requires pyarrow. Dataset bytes stay in the user's cache. Only questions are
sent to RustXNG; reference answers and URLs are used exclusively for scoring.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

REVISION = "0c298f519c871579c75920189b3858da29fe2c81"
DATASET = "https://huggingface.co/datasets/vtllms/sealqa"


def canonical_page(url: str) -> tuple[str, str, str]:
    """Ignore scheme, fragment, www and trailing slash; keep path case/query.

    Reference text fragments identify passages, not different pages. This does
    not allow prefix matches, directory descendants or arbitrary query removal.
    """
    try:
        parts = urllib.parse.urlsplit(url)
        host = parts.hostname
        port = parts.port
    except ValueError:
        return ("", "", "")
    if parts.scheme.lower() not in ("http", "https") or not host:
        return ("", "", "")
    host = host.lower().removeprefix("www.")
    if port and port not in (80, 443):
        host += f":{port}"
    path = urllib.parse.unquote(parts.path).rstrip("/") or "/"
    return host, path, parts.query


def rank_of(urls: list[str], references: list[str]) -> int | None:
    gold = {canonical_page(url) for url in references} - {("", "", "")}
    return next((i for i, url in enumerate(urls, 1) if canonical_page(url) in gold), None)


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[max(0, min(len(ordered) - 1, int((len(ordered) - 1) * fraction + 0.5)))]


def summarise(rows: list[dict]) -> dict:
    total = len(rows)
    times = [row["elapsed_ms"] for row in rows]
    return {
        "questions": total,
        **{f"reference_hit@{k}": sum(row["rank"] is not None and row["rank"] <= k for row in rows) for k in (1, 5, 10)},
        **{f"reference_hit@{k}_pct": round(100 * sum(row["rank"] is not None and row["rank"] <= k for row in rows) / total, 2) if total else 0 for k in (1, 5, 10)},
        "reference_mrr@10": round(sum(1 / row["rank"] for row in rows if row["rank"] is not None and row["rank"] <= 10) / total, 4) if total else 0,
        "errors": sum(bool(row.get("error")) for row in rows),
        "zero_results": sum(not row.get("urls") for row in rows),
        "median_ms": round(statistics.median(times), 1) if times else 0,
        "p95_ms": round(percentile(times, 0.95), 1),
        "median_results": statistics.median([len(row.get("urls", [])) for row in rows]) if rows else 0,
        "median_unresponsive": statistics.median([row.get("unresponsive_count", 0) for row in rows]) if rows else 0,
        "benchmark_mirror_results": sum(len(row.get("benchmark_mirrors", [])) for row in rows),
    }


def search(base: str, question: str, timeout: float) -> dict:
    query = urllib.parse.urlencode({"q": question, "format": "json", "max_results": 10, "full": 1})
    request = urllib.request.Request(base.rstrip("/") + "/search?" + query, headers={"User-Agent": "rustxng-sealqa-retrieval-eval/1.0"})
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read()
        payload = json.loads(body)
        urls = [item["url"] for item in payload.get("results", [])][:10]
        return {
            "urls": urls,
            "results": [{key: item.get(key) for key in ("url", "title", "engine", "engines", "score")} for item in payload.get("results", [])][:10],
            "unresponsive": payload.get("unresponsive_engines", []),
            "unresponsive_count": len(payload.get("unresponsive_engines", [])),
            "bytes": len(body),
            "error": payload.get("error"),
            "benchmark_mirrors": [url for url in urls if any(token in url.lower() for token in ("sealqa", "seal-0", "seal-hard", "2506.01062"))],
            "elapsed_ms": round((time.perf_counter() - started) * 1000, 2),
        }
    except (urllib.error.URLError, TimeoutError, ValueError, KeyError) as error:
        return {"urls": [], "error": f"{type(error).__name__}: {error}", "elapsed_ms": round((time.perf_counter() - started) * 1000, 2)}


def main() -> None:
    import pyarrow.parquet as pq

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="http://127.0.0.1:18991")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cache", type=Path, default=Path.home() / ".cache/rustxng-sealqa" / REVISION)
    parser.add_argument("--delay", type=float, default=0.25)
    parser.add_argument("--timeout", type=float, default=20)
    args = parser.parse_args()
    args.cache.mkdir(parents=True, exist_ok=True)
    args.output.mkdir(parents=True, exist_ok=True)
    datasets = {}
    files = {}
    for split, name in (("seal_0", "seal-0"), ("seal_hard", "seal-hard")):
        path = args.cache / f"{name}.parquet"
        if not path.exists():
            urllib.request.urlretrieve(f"{DATASET}/resolve/{REVISION}/{path.name}", path)
        files[split] = {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "file": path.name}
        datasets[split] = pq.read_table(path).to_pylist()

    # Seal-0 is a subset of Seal-Hard in this revision: fetch each distinct
    # question once, then reuse that observation in both split summaries.
    unique = {}
    for rows in datasets.values():
        for row in rows:
            unique.setdefault(row["question"], row)

    metadata = {
        "dataset": DATASET, "dataset_revision": REVISION, "dataset_files": files,
        "started_at": datetime.now(timezone.utc).isoformat(), "base": args.base,
        "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "settings_sha256": hashlib.sha256(Path("settings.toml").read_bytes()).hexdigest(),
        "settings_sha256_scope": "repository settings.toml only; not the effective profile, overrides or index contents",
        "worktree_has_tracked_changes": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], text=True).strip()),
        "index_state": "not determined by this evaluator; document actual server configuration and index artifacts separately",
        "protocol": "single question verbatim; default parallel strategy; max_results=10; no LLM requested by evaluator; no raw content requested; local index presence not checked; one persistent server; serial requests; shared observation for overlapping split questions",
        "scoring": "exact canonical page; scheme/fragment/www/trailing slash ignored; path case and query preserved; no URL-prefix match; not official QA accuracy",
        "delay_seconds": args.delay, "client_timeout_seconds": args.timeout,
        "unique_questions": len(unique),
    }
    (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    observations = {}
    with (args.output / "observations.jsonl").open("w") as out:
        for index, (question, row) in enumerate(unique.items(), 1):
            result = search(args.base, question, args.timeout)
            result.update({"question": question, "reference_urls": row["urls"], "freshness": row["freshness"], "effective_year": row["effective_year"], "topic": row["topic"]})
            result["rank"] = rank_of(result["urls"], row["urls"])
            observations[question] = result
            out.write(json.dumps(result, ensure_ascii=False) + "\n")
            out.flush()
            print(f"{index:3}/{len(unique)} rank={str(result['rank']):>4} {result['elapsed_ms']:8.1f} ms n={len(result['urls']):2} errors={bool(result.get('error'))}", flush=True)
            if args.delay:
                time.sleep(args.delay)

    report = {"metadata": metadata, "completed_at": datetime.now(timezone.utc).isoformat(), "splits": {}}
    for split, rows in datasets.items():
        scored = []
        for row in rows:
            observation = dict(observations[row["question"]])
            observation["rank"] = rank_of(observation["urls"], row["urls"])
            scored.append(observation)
        report["splits"][split] = summarise(scored)
    (args.output / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report["splits"], indent=2), flush=True)


if __name__ == "__main__":
    main()
