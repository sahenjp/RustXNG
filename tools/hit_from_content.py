#!/usr/bin/env python3
"""Compute hit@1 / hit@5 / MRR from collected bench/content-*.json files.

All backends were queried with the same 20 questions and gold URL prefixes, so
this is the apples-to-apples URL-quality comparison (no live endpoints needed).

Usage:
    python3 tools/hit_from_content.py --top 10
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from reference_urls import first_hit_rank


def rank(results: list[dict], gold: str | list[str]) -> int | None:
    return first_hit_rank([str(result.get("url", "")) for result in results], [gold] if isinstance(gold, str) else gold)


def gold_by_id() -> dict:
    """Gold URLs from the question file, so a fixed gold applies to every
    backend without re-collecting them."""
    path = Path("bench/quality-questions.json")
    if not path.exists():
        return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    return {q["id"]: q["gold"] for q in data.get("queries", []) if q.get("gold")}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--top", type=int, default=10)
    args = parser.parse_args()

    files = [
        ("RustXNG", "bench/content-rustxng.json"),
        ("SearXNG", "bench/content-searxng.json"),
        ("Tavily", "bench/content-tavily.json"),
        ("Exa", "bench/content-exa.json"),
        ("Ollama", "bench/content-ollama.json"),
    ]
    header = f"{'backend':<10} {'hit@1':>7} {'hit@5':>7} {'hit@10':>7} {'MRR':>6} {'n':>3}"
    print(f"URL 中率（上位 {args.top} 件・20 問・gold URL 構造・境界一致）\n")
    print(header)
    print("-" * len(header))
    for name, rel in files:
        path = Path(rel)
        if not path.exists():
            print(f"{name:<10} {'(未収集)':>7}")
            continue
        data = json.loads(path.read_text(encoding="utf-8"))
        golds = gold_by_id()
        ranks = []
        for q in data["queries"]:
            gold = golds.get(q["id"]) or [q["gold_url"]]
            rk = rank((q.get("results") or [])[: args.top], gold)
            ranks.append(rk)
        n = len(ranks)
        hit1 = sum(1 for r in ranks if r == 1)
        hit5 = sum(1 for r in ranks if r is not None and r <= 5)
        hit10 = sum(1 for r in ranks if r is not None and r <= 10)
        mrr = sum(1 / r for r in ranks if r is not None) / n
        print(f"{name:<10} {hit1:>4}/{n:<2} {hit5:>4}/{n:<2} {hit10:>4}/{n:<2} {mrr:>6.3f} {n:>3}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
