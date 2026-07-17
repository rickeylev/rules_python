import os
import pathlib
import shutil
import tempfile
import unittest
from unittest.mock import MagicMock, patch

from tools.private.release.gh import (
    MultipleTrackingIssuesError,
    NoTrackingIssueError,
)
from tools.private.release.mock_gh import MockGitHub


def _mock_git(test_case):
    mock_git = MagicMock()
    test_case.mock_git = mock_git

    # Mock Git inside utils.py since it instantiates it locally
    patch("tools.private.release.utils.Git", return_value=mock_git).start()

    test_case.addCleanup(patch.stopall)

    # Apply safe defaults
    mock_git.get_current_branch.return_value = None
    mock_git.get_tags.return_value = []
    mock_git.get_remote_tags.return_value = []
    mock_git.status.return_value = ""
    mock_git.branch_exists.return_value = False
    mock_git.tag_exists.return_value = False
    return mock_git


def _mock_git_and_gh(test_case):
    _mock_git(test_case)
    mock_gh = MagicMock()
    test_case.mock_gh = mock_gh

    mock_gh.MultipleTrackingIssuesError = MultipleTrackingIssuesError
    mock_gh.NoTrackingIssueError = NoTrackingIssueError

    mock_gh.get_release_tracking_issue.side_effect = NoTrackingIssueError("Not found")
    mock_gh.get_open_pr.return_value = None


class TempDirTestCase(unittest.TestCase):
    def setUp(self):
        self.tmpdir = pathlib.Path(tempfile.mkdtemp())
        self.original_cwd = os.getcwd()
        self.addCleanup(shutil.rmtree, self.tmpdir)
        os.chdir(self.tmpdir)
        self.addCleanup(os.chdir, self.original_cwd)


DEFAULT_RELEASE_TEMPLATE_CONTENT = (
    "template content\n"
    "- [ ] Prepare Release\n"
    "- [ ] Tag RC0\n"
    "- [ ] Tag Final\n"
    "\n"
    "## Backports\n"
)


class ReleaseToolTestCase(TempDirTestCase):
    def setUp(self):
        super().setUp()
        self.gh = MockGitHub()
        self.setUpReleaseTemplate()

    def setUpReleaseTemplate(self):
        template_dir = self.tmpdir / ".github/ISSUE_TEMPLATE"
        template_dir.mkdir(parents=True, exist_ok=True)
        self.template_file = template_dir / "release_tracking_template.md"
        self.template_content = DEFAULT_RELEASE_TEMPLATE_CONTENT
        self.template_file.write_text(self.template_content, encoding="utf-8")
