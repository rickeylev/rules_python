import argparse
import unittest

from tests.tools.private.release.release_test_helper import ReleaseToolTestCase
from tools.private.release.backport_create_releases import BackportCreateReleases


class CmdBackportCreateReleasesTest(ReleaseToolTestCase):
    def test_create_releases_all_success(self):
        # Arrange
        args = argparse.Namespace(issue=123, dry_run=False)
        gh = self.gh

        # Setup backport issue in mock GH
        backport_body = """* PR: #456
* From version: 1.7
* To version: 1.9

## Tasks

- [x] Verify apply 1.7 | status=success
- [x] Verify apply 1.8 | status=success
- [x] Verify apply 1.9 | status=success
- [ ] Track Release 1.7.2
- [ ] Track Release 1.8.1
- [ ] Track Release 1.9.0"""

        gh.issues[123] = {
            "title": "Backport: #456",
            "body": backport_body,
            "labels": ["type:backport-pr"],
            "number": 123,
            "url": "https://github.com/.../issues/123",
        }

        # Act
        result = BackportCreateReleases(args, gh).run()

        # Assert
        self.assertEqual(result, 0)

        # Verify release issues created (IDs 1001, 1002, 1003)
        self.assertIn(1001, gh.issues)
        self.assertIn(1002, gh.issues)
        self.assertIn(1003, gh.issues)

        # 1.7.2 (patch) should not have Tag RC0
        self.assertEqual(gh.issues[1001]["title"], "Release 1.7.2")
        self.assertNotIn("Tag RC0", gh.issues[1001]["body"])
        self.assertIn("## Backports\n- [ ] #456", gh.issues[1001]["body"])

        # 1.9.0 (minor) should have Tag RC0
        self.assertEqual(gh.issues[1003]["title"], "Release 1.9.0")
        self.assertIn("Tag RC0", gh.issues[1003]["body"])

        # Verify backport issue updated
        expected_updated_backport_body = """* PR: #456
* From version: 1.7
* To version: 1.9

## Tasks

- [x] Verify apply 1.7 | status=success
- [x] Verify apply 1.8 | status=success
- [x] Verify apply 1.9 | status=success
- [x] Track Release 1.7.2 | status=success release_issue=#1001
- [x] Track Release 1.8.1 | status=success release_issue=#1002
- [x] Track Release 1.9.0 | status=success release_issue=#1003"""

        self.assertEqual(gh.issues[123]["body"], expected_updated_backport_body)

    def test_create_releases_dependency_blocking(self):
        # Arrange
        args = argparse.Namespace(issue=123, dry_run=False)
        gh = self.gh

        # 1.8 failed, 1.7 and 1.9 succeeded
        backport_body = """* PR: #456
* From version: 1.7
* To version: 1.9

## Tasks

- [x] Verify apply 1.7 | status=success
- [ ] Verify apply 1.8 | status=failed-conflict
- [x] Verify apply 1.9 | status=success
- [ ] Track Release 1.7.2
- [ ] Track Release 1.8.1
- [ ] Track Release 1.9.0"""

        gh.issues[123] = {
            "title": "Backport: #456",
            "body": backport_body,
            "labels": ["type:backport-pr"],
            "number": 123,
            "url": "https://github.com/.../issues/123",
        }

        # Act
        result = BackportCreateReleases(args, gh).run()

        # Assert
        self.assertEqual(result, 0)

        # Only 1.9.0 (ID 1001) should be created. 1.7.2 and 1.8.1 are blocked by 1.8 failure.
        self.assertEqual(len(gh.issues), 2)  # Backport issue + 1 release issue
        self.assertIn(1001, gh.issues)
        self.assertEqual(gh.issues[1001]["title"], "Release 1.9.0")

        # Verify backport issue updated with block statuses
        expected_updated_backport_body = """* PR: #456
* From version: 1.7
* To version: 1.9

## Tasks

- [x] Verify apply 1.7 | status=success
- [ ] Verify apply 1.8 | status=failed-conflict
- [x] Verify apply 1.9 | status=success
- [ ] Track Release 1.7.2 | status=error-later-release-did-not-apply
- [ ] Track Release 1.8.1 | status=error-later-release-did-not-apply
- [x] Track Release 1.9.0 | status=success release_issue=#1001"""

        self.assertEqual(gh.issues[123]["body"], expected_updated_backport_body)

    def test_create_releases_already_exists(self):
        # Arrange
        args = argparse.Namespace(issue=123, dry_run=False)
        gh = self.gh

        backport_body = """* PR: #456
* From version: 1.7
* To version: 1.7

## Tasks

- [x] Verify apply 1.7 | status=success
- [ ] Track Release 1.7.2"""

        gh.issues[123] = {
            "title": "Backport: #456",
            "body": backport_body,
            "labels": ["type:backport-pr"],
            "number": 123,
            "url": "https://github.com/.../issues/123",
        }

        # Pre-create the release issue to simulate it already existing
        gh.create_issue(
            title="Release 1.7.2",
            body="existing body\n## Backports\n",
            labels=["type: release"],
        )  # Will get ID 1001

        # Act
        result = BackportCreateReleases(args, gh).run()

        # Assert
        self.assertEqual(result, 0)
        self.assertEqual(len(gh.issues), 2)

        # Should link existing issue 1001
        expected_updated_backport_body = """* PR: #456
* From version: 1.7
* To version: 1.7

## Tasks

- [x] Verify apply 1.7 | status=success
- [x] Track Release 1.7.2 | status=success release_issue=#1001"""

        self.assertEqual(gh.issues[123]["body"], expected_updated_backport_body)


if __name__ == "__main__":
    unittest.main()
