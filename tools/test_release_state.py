import unittest

from release_state import plan


SHA = 'a' * 40


class ReleaseStateTests(unittest.TestCase):
    def published(self):
        return {'tag_name': 'v0.27.1', 'draft': False, 'prerelease': False}

    def draft(self):
        return {'tag_name': 'v0.27.1', 'draft': True,
                'author': {'login': 'github-actions[bot]'}, 'target_commitish': SHA}

    def test_new_draft_publishes_without_preparing_another_pr(self):
        result = plan('0.27.1', '0.27.1', [], False, True, 'v0.27.1', SHA)
        self.assertEqual((result['created'], result['prepare_pr'], result['sha']), ('true', 'false', SHA))

    def test_unpublished_draft_without_a_tag_resumes_publication(self):
        result = plan('0.27.1', '0.27.1', [self.draft()], False)
        self.assertEqual((result['created'], result['prepare_pr']), ('true', 'false'))

    def test_only_a_published_baseline_and_tag_allow_pr_preparation(self):
        result = plan('0.27.1', '0.27.1', [self.published()], True)
        self.assertEqual((result['created'], result['prepare_pr']), ('false', 'true'))

    def test_missing_release_or_tag_never_falls_back_to_old_history(self):
        for releases, exists in [([], False), ([], True), ([self.published()], False)]:
            with self.subTest(releases=releases, exists=exists), self.assertRaises(ValueError):
                plan('0.27.1', '0.27.1', releases, exists)

    def test_version_mismatch_is_rejected(self):
        with self.assertRaises(ValueError):
            plan('0.27.1', '0.27.0', [self.published()], True)

    def test_malformed_or_unexpected_action_outputs_are_rejected(self):
        for created, tag, sha in [(True, '', ''), (True, 'v0.28.0', SHA),
                                  (True, 'v0.27.1', 'main'), (False, 'v0.27.1', SHA)]:
            with self.subTest(created=created, tag=tag, sha=sha), self.assertRaises(ValueError):
                plan('0.27.1', '0.27.1', [], False, created, tag, sha)

    def test_ambiguous_or_untrusted_drafts_do_not_prepare_a_pr(self):
        with self.assertRaises(ValueError):
            plan('0.27.1', '0.27.1', [self.draft(), self.draft()], False)
        draft = self.draft()
        draft['author']['login'] = 'someone-else'
        with self.assertRaises(ValueError):
            plan('0.27.1', '0.27.1', [draft], False)


if __name__ == '__main__':
    unittest.main()
