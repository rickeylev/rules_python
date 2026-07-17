"""In-memory fake for GitHub API."""

import re

from tools.private.release.gh import RELEASE_LABEL


class MockGitHub:
    def __init__(self, repo: str = "bazel-contrib/rules_python"):
        self.repo = repo
        self.issues = {}
        self.next_issue_num = 1001
        self.prs = {}  # num -> pr_info

    def create_issue(
        self, title: str, body: str, labels: list[str] | None = None
    ) -> int:
        issue_num = self.next_issue_num
        self.next_issue_num += 1
        self.issues[issue_num] = {
            "title": title,
            "body": body,
            "labels": labels or [],
            "number": issue_num,
            "url": f"https://github.com/{self.repo}/issues/{issue_num}",
        }
        return issue_num

    def create_tracking_issue(self, version: str, template_content: str) -> int:
        # Strip YAML frontmatter if present (simplified copy from gh.py)
        issue_body = template_content
        if template_content.startswith("---"):
            parts = template_content.split("---", 2)
            if len(parts) >= 3:
                issue_body = parts[2].strip()

        return self.create_issue(
            title=f"Release {version}", body=issue_body, labels=[RELEASE_LABEL]
        )

    def get_issue_body(self, issue_num: int) -> str:
        if issue_num not in self.issues:
            raise ValueError(f"Issue #{issue_num} not found in MockGitHub")
        return self.issues[issue_num]["body"]

    def update_issue_body(self, issue_num: int, body: str):
        if issue_num not in self.issues:
            raise ValueError(f"Issue #{issue_num} not found in MockGitHub")
        self.issues[issue_num]["body"] = body

    def resolve_pr_number(self, pr_ref: str) -> int:
        # Real algorithm copy (doesn't require RPCs)
        clean_ref = pr_ref.lstrip("#")
        if clean_ref.isdigit():
            return int(clean_ref)

        if pr_ref.startswith("http"):
            pattern = rf"github\.com/{re.escape(self.repo)}/pull/(\d+)(/|\?|\Z)"
            match = re.search(pattern, pr_ref, re.IGNORECASE)
            if match:
                return int(match.group(1))
            raise ValueError(
                f"URL is not for the configured repository ({self.repo}): {pr_ref}"
            )
        raise ValueError(f"Could not resolve PR ref: {pr_ref}")

    def get_open_tracking_issues(self, version: str | None = None) -> list[dict]:
        results = []
        for issue in self.issues.values():
            if RELEASE_LABEL in issue["labels"]:
                if version:
                    if issue["title"] == f"Release {version}":
                        results.append(issue)
                else:
                    results.append(issue)
        return results

    def get_pr_info(self, pr_num: int) -> dict:
        if pr_num in self.prs:
            return self.prs[pr_num]
        return {
            "state": "MERGED",
            "mergeCommit": {"oid": f"mock_merge_sha_{pr_num}"},
        }
