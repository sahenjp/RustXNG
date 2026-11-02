"""Offline fixture tests for Japanese benchmark scoring and query construction."""

import json
import unittest
from unittest.mock import patch
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

from eval_japanese import broad_metrics, canonical_page, make_query, pinpoint_summary, rank_of


class JapaneseBenchmarkTests(unittest.TestCase):
    def test_corpus_has_30_pinned_official_japanese_diagnostics(self):
        corpus = json.loads((Path(__file__).resolve().parents[1] / "bench/japanese-search/diagnostics.json").read_text(encoding="utf-8"))
        self.assertEqual(len(corpus["cases"]), 30)
        for case in corpus["cases"]:
            self.assertEqual(case["typescript_version"], "5.9.3")
            self.assertIn("microsoft/TypeScript/blob/v5.9.3/lib/ja/diagnosticMessages.generated.json", case["source_url"])
            self.assertTrue(case["expected_reference_urls"])
            self.assertTrue(case["exact_error"])
            instantiated = case["diagnostic_message_template"]
            for index, value in enumerate(case["template_arguments"]):
                instantiated = instantiated.replace("{" + str(index) + "}", value)
            self.assertEqual(instantiated, case["exact_error"])

    def test_transforms_are_distinct_and_recordable(self):
        case = {"exact_error": "名前 'x' が見つかりません。", "keyword_terms": ["名前", "x"]}
        self.assertEqual(make_query(case, "raw"), "名前 'x' が見つかりません。")
        self.assertEqual(make_query(case, "quoted"), '"名前 \'x\' が見つかりません。"')
        self.assertEqual(make_query(case, "keywords"), "名前 x")

    def test_qiita_and_language_are_independent_request_conditions(self):
        from eval_japanese import build_search_url
        baseline = parse_qs(urlsplit(build_search_url("http://localhost", "query", "default", False, None, False, baseline_engines=["duckduckgo", "bing"])).query)
        language = parse_qs(urlsplit(build_search_url("http://localhost", "query", "ja", False, None, False, baseline_engines=["duckduckgo", "bing"])).query)
        qiita = parse_qs(urlsplit(build_search_url("http://localhost", "query", "default", True, None, False, baseline_engines=["duckduckgo", "bing"])).query)
        self.assertNotIn("language", baseline)
        self.assertEqual(language["language"], ["ja"])
        self.assertEqual(baseline["engines"], ["duckduckgo,bing"])
        self.assertEqual(language["engines"], ["duckduckgo,bing"])
        self.assertEqual(qiita["engines"], ["duckduckgo,bing,qiita"])
        self.assertNotIn("language", qiita)

    def test_upstream_camel_case_publication_date_is_preserved(self):
        from eval_japanese import search
        from io import BytesIO
        response = BytesIO(json.dumps({"results": [{"url": "https://example.org/", "publishedDate": "2026-11-02"}]}).encode())
        response.status = 200
        with patch("urllib.request.urlopen", return_value=response):
            row = search("http://localhost", "test", "ja", False, None, False, 1)
        self.assertEqual(row["results"][0]["published_date"], "2026-11-02")

    def test_canonical_rank_requires_same_page(self):
        self.assertEqual(rank_of(["https://www.example.org/a/#frag"], ["http://example.org/a"]), 1)
        self.assertIsNone(rank_of(["https://example.org/a-extra"], ["https://example.org/a"]))
        self.assertEqual(rank_of(["https://example.org/%E6%9D%B1%E4%BA%AC"], ["https://example.org/東京"]), 1)
        self.assertEqual(canonical_page("https://example.org/a"), ("example.org", "/a", ""))

    def test_pinpoint_summary_counts_failures_and_latency(self):
        result = pinpoint_summary([
            {"rank": 1, "elapsed_ms": 50}, {"rank": 5, "elapsed_ms": 150},
            {"rank": None, "elapsed_ms": 250, "error": "timeout"},
        ])
        self.assertEqual(result["reference_hit@1"], 1)
        self.assertEqual(result["reference_hit@5"], 2)
        self.assertEqual(result["failures"], 1)
        self.assertEqual(result["request_errors"], 1)
        self.assertEqual(result["p50_ms"], 150)

    def test_broad_metrics_measure_diversity_duplicates_and_date_coverage(self):
        row = {
            "target_source_domains": ["a.example", "b.example"],
            "results": [
                {"url": "https://a.example/1", "published_date": "2026-10-30T12:00:00Z"},
                {"url": "https://a.example/1#fragment", "published_date": None},
                {"url": "https://b.example/2", "published_date": "2020-01-01"},
                {"url": "https://c.example/3", "published_date": "not-a-date"},
            ],
        }
        result = broad_metrics(row, 30)
        self.assertEqual(result["unique_domains"], 3)
        self.assertEqual(result["duplicate_pages"], 1)
        self.assertEqual(result["publication_date_count"], 2)
        self.assertEqual(result["publication_date_missing_count"], 1)
        self.assertEqual(result["publication_date_invalid_count"], 1)
        self.assertEqual(result["target_source_domain_coverage"], 1.0)
        self.assertGreaterEqual(result["within_time_window_count"], 1)


if __name__ == "__main__":
    unittest.main()
