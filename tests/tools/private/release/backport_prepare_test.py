import argparse
import unittest
from unittest.mock import call, patch

from tests.tools.private.release.release_test_helper import (
    ReleaseToolTestCase,
    _mock_git,
)
from tools.private.release.backport_prepare import BackportPrepare
from tools.private.release.gh import BACKPORT_LABEL


class CmdBackportPrepareTest(ReleaseToolTestCase):
    def setUp(self):
        super().setUp()
        _mock_git(self)
        # Mock changelog_news and determine_next_version
        self.patcher_news = patch(
            "tools.private.release.backport_prepare.changelog_news"
        )
        self.mock_news = self.patcher_news.start()

        self.patcher_det = patch(
            "tools.private.release.backport_prepare.determine_next_version"
        )
        self.mock_det = self.patcher_det.start()

        self.addCleanup(self.patcher_news.stop)
        self.addCleanup(self.patcher_det.stop)

    def test_prepare_from_issue_success(self):
        # Arrange
        args = argparse.Namespace(
            issue=123,
            pr=None,
            from_minor=None,
            to_minor=None,
            remote="my-remote",
            dry_run=False,
        )

        # Setup backport issue in mock GH
        backport_body = "* PR: #456\n* From version: 1.7\n* To version: 1.9\n"
        self.gh.issues[123] = {
            "title": "Backport: #456",
            "body": backport_body,
            "labels": ["type: backport-pr"],
            "number": 123,
            "url": "https://github.com/.../issues/123",
        }

        # Setup PR info in mock GH
        self.gh.prs[456] = {
            "state": "MERGED",
            "mergeCommit": {"oid": "pr_merge_sha_12345"},
        }

        # Mock remote branches
        self.mock_git.get_remote_branches.return_value = [
            "main",
            "release/1.6",
            "release/1.7",
            "release/1.8",
            "release/1.9",
            "release/2.0",
        ]
        self.mock_git.get_current_branch.return_value = "work-branch"

        # Mock next versions
        self.mock_det.side_effect = ["1.7.2", "1.8.1", "1.9.0"]

        # Act
        result = BackportPrepare(args, self.mock_git, self.gh).run()

        # Assert
        self.assertEqual(result, 0)
        self.mock_git.fetch.assert_called_once_with("my-remote", tags=True, force=True)

        # Verify checkouts and cherry-picks
        self.mock_git.checkout.assert_has_calls(
            [
                call("release/1.7", track_remote="my-remote"),
                call("release/1.8", track_remote="my-remote"),
                call("release/1.9", track_remote="my-remote"),
                call("work-branch"),  # Restored branch
            ]
        )

        self.mock_git.cherry_pick.assert_has_calls(
            [
                call("pr_merge_sha_12345"),
                call("pr_merge_sha_12345"),
                call("pr_merge_sha_12345"),
            ]
        )

        # Verify changelog updates
        self.mock_news.update_changelog.assert_has_calls(
            [
                call("1.7.2", unittest.mock.ANY),
                call("1.8.1", unittest.mock.ANY),
                call("1.9.0", unittest.mock.ANY),
            ]
        )

        # Verify issue body update
        expected_body = (
            "* PR: #456\n"
            "* From version: 1.7\n"
            "* To version: 1.9\n"
            "\n"
            "## Tasks\n"
            "\n"
            "- [x] Verify apply 1.7 | status=success\n"
            "- [x] Verify apply 1.8 | status=success\n"
            "- [x] Verify apply 1.9 | status=success\n"
            "- [ ] Track Release 1.7.2\n"
            "- [ ] Track Release 1.8.1\n"
            "- [ ] Track Release 1.9.0"
        )
        self.assertEqual(self.gh.issues[123]["body"], expected_body)

    def test_prepare_manual_success(self):
        # Arrange
        args = argparse.Namespace(
            issue=None,
            pr="#456",
            from_minor="1.7",
            to_minor="1.8",
            remote="my-remote",
            dry_run=False,
        )
        self.gh.prs[456] = {
            "state": "MERGED",
            "mergeCommit": {"oid": "pr_merge_sha_12345"},
        }
        self.mock_git.get_remote_branches.return_value = [
            "release/1.7",
            "release/1.8",
        ]
        self.mock_git.get_current_branch.return_value = "work-branch"
        self.mock_det.side_effect = ["1.7.2", "1.8.1"]

        # Act
        result = BackportPrepare(args, self.mock_git, self.gh).run()

        # Assert
        self.assertEqual(result, 0)
        self.assertIn(1001, self.gh.issues)
        issue = self.gh.issues[1001]
        self.assertEqual(issue["title"], "Backport: #456")
        self.assertEqual(issue["labels"], [BACKPORT_LABEL])
        body = issue["body"]
        self.assertIn("- [x] Verify apply 1.7 | status=success", body)
        self.assertIn("- [x] Verify apply 1.8 | status=success", body)
        self.assertIn("- [ ] Track Release 1.7.2", body)
        self.assertIn("- [ ] Track Release 1.8.1", body)

    def test_prepare_manual_with_patch_versions(self):
        # Arrange
        args = argparse.Namespace(
            issue=None,
            pr="#456",
            from_minor="1.7.0",
            to_minor="1.8.0",
            remote="my-remote",
            dry_run=False,
        )
        self.gh.prs[456] = {
            "state": "MERGED",
            "mergeCommit": {"oid": "pr_merge_sha_12345"},
        }
        self.mock_git.get_remote_branches.return_value = [
            "release/1.7",
            "release/1.8",
        ]
        self.mock_git.get_current_branch.return_value = "work-branch"
        self.mock_det.side_effect = ["1.7.2", "1.8.1"]

        # Act
        result = BackportPrepare(args, self.mock_git, self.gh).run()

        # Assert
        self.assertEqual(result, 0)
        self.assertIn(1001, self.gh.issues)
        body = self.gh.issues[1001]["body"]
        self.assertIn("- [x] Verify apply 1.7 | status=success", body)
        self.assertIn("- [x] Verify apply 1.8 | status=success", body)

    def test_prepare_verify_failed(self):
        # Arrange
        args = argparse.Namespace(
            issue=123,
            pr=None,
            from_minor=None,
            to_minor=None,
            remote="my-remote",
            dry_run=False,
        )
        issue_body = "* PR: #456\n* From version: 1.7\n* To version: 1.8\n"
        self.gh.issues[123] = {
            "title": "Backport: #456",
            "body": issue_body,
            "labels": ["type: backport-pr"],
            "number": 123,
            "url": "https://github.com/.../issues/123",
        }
        self.gh.prs[456] = {
            "state": "MERGED",
            "mergeCommit": {"oid": "pr_merge_sha_12345"},
        }
        self.mock_git.get_remote_branches.return_value = [
            "release/1.7",
            "release/1.8",
        ]
        self.mock_git.get_current_branch.return_value = "work-branch"
        self.mock_det.side_effect = ["1.7.2", "1.8.1"]

        # Mock cherry-pick failure on 1.7 and changelog failure on 1.8
        self.mock_git.cherry_pick.side_effect = [Exception("Conflict"), None]
        self.mock_news.update_changelog.side_effect = [Exception("Changelog error")]

        # Act
        result = BackportPrepare(args, self.mock_git, self.gh).run()

        # Assert
        self.assertEqual(
            result, 0
        )  # Returns 0 even if verification fails, it just updates tasks

        expected_body = (
            "* PR: #456\n"
            "* From version: 1.7\n"
            "* To version: 1.8\n"
            "\n"
            "## Tasks\n"
            "\n"
            "- [ ] Verify apply 1.7 | status=failed-conflict\n"
            "- [ ] Verify apply 1.8 | status=failed-changelog\n"
            "- [ ] Track Release 1.7.2\n"
            "- [ ] Track Release 1.8.1"
        )
        self.assertEqual(self.gh.issues[123]["body"], expected_body)


if __name__ == "__main__":
    unittest.main()
