import unittest

from reference_urls import first_hit_rank, matches_reference


class ReferenceUrlTests(unittest.TestCase):
    def test_path_and_authority_boundaries(self):
        gold = "https://example.com/docs"
        for url in ["https://example.com/docs-malicious", "https://example.com.evil/docs", "https://example.com:8443/docs", "javascript:example.com/docs"]:
            self.assertFalse(matches_reference(url, gold), url)
        for url in [gold, gold + "/", gold + "/guide", gold + ".html", gold + "?view=all"]:
            self.assertTrue(matches_reference(url, gold), url)

    def test_case_and_reserved_encoding_remain_distinct(self):
        self.assertFalse(matches_reference("https://e.test/Docs", "https://e.test/docs"))
        self.assertFalse(matches_reference("https://e.test/a%2Fb", "https://e.test/a/b"))
        self.assertFalse(matches_reference("https://e.test/docs/../evil", "https://e.test/docs"))
        self.assertFalse(matches_reference("https://e.test/docs/%2e%2e/evil", "https://e.test/docs"))
        self.assertTrue(matches_reference("https://e.test/%64ocs", "https://e.test/docs"))

    def test_reference_query_is_required(self):
        self.assertFalse(matches_reference("https://e.test/?id=12", "https://e.test/?id=1"))
        self.assertTrue(matches_reference("https://e.test/?id=1#x", "https://e.test/?id=1"))

    def test_unicode_and_percent_encoded_paths_are_equivalent(self):
        self.assertTrue(matches_reference("https://ja.wikipedia.org/wiki/%E5%AF%8C%E5%A3%AB%E5%B1%B1", "https://ja.wikipedia.org/wiki/富士山"))
        self.assertFalse(matches_reference("https://ja.wikipedia.org/wiki/源氏物語各帖のあらすじ", "https://ja.wikipedia.org/wiki/源氏物語"))

    def test_legacy_version_aliases_and_html_extension_are_preserved(self):
        self.assertTrue(matches_reference("http://docs.python.org/3.14/library/stdtypes.html#dict", "https://docs.python.org/3/library/stdtypes"))
        self.assertTrue(matches_reference("https://e.test/en/current/guide", "https://e.test/en/stable/guide"))

    def test_malformed_inputs_and_first_rank(self):
        for url in ["", "https://", "https://e.test:bad/docs", "https://user:pass@e.test/docs"]:
            self.assertFalse(matches_reference(url, "https://e.test/docs"))
        self.assertEqual(first_hit_rank(["https://e.test/docs-malicious", "https://e.test/docs"], ["https://e.test/docs"]), 2)


if __name__ == "__main__":
    unittest.main()
