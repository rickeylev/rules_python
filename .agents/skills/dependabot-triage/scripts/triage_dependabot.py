#!/usr/bin/env python3
"""Triage and categorize Dependabot findings in rules_python into Easy vs Hard.

Produces a categorized report mapping each Dependabot ID to its category:
- EASY_INTERNAL: Internal tools, dev dependencies, docs, tests, or examples.
- HARD_PUBLIC_API: Public APIs, core Bazel rules, or exported behaviors.
- UNREFERENCED_TRANSITIVE: Pulled in indirectly without a manifest.
"""

import argparse
import asyncio
from collections.abc import AsyncIterator
from enum import Enum
import http.client
import io
import json
import os
from pathlib import Path
import re
import sys
from typing import Optional, TypedDict


REPO = "bazel-contrib/rules_python"

INTERNAL_SUBSTRINGS = (
    "examples/",
    "tests/",
    "docs/",
    "dev/",
    "tools/",
    "benchmarks/",
    "gazelle/examples/",
    "sphinxdocs/dev/",
)


class PackageDict(TypedDict, total=False):
    """Package information in a Dependabot alert.

    See: https://docs.github.com/en/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository
    """

    ecosystem: str
    name: str


class DependencyDict(TypedDict, total=False):
    """Dependency and manifest reference in a Dependabot alert.

    See: https://docs.github.com/en/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository
    """

    package: PackageDict
    manifest_path: str
    scope: Optional[str]
    relationship: Optional[str]


class PatchedVersionDict(TypedDict, total=False):
    """Patched version identifier.

    See: https://docs.github.com/en/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository
    """

    identifier: str


class SecurityVulnerabilityDict(TypedDict, total=False):
    """Security vulnerability details in a Dependabot alert.

    See: https://docs.github.com/en/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository
    """

    package: PackageDict
    severity: str
    vulnerable_version_range: str
    first_patched_version: Optional[PatchedVersionDict]


class SecurityAdvisoryDict(TypedDict, total=False):
    """Security advisory details in a Dependabot alert.

    See: https://docs.github.com/en/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository
    """

    ghsa_id: str
    cve_id: Optional[str]
    summary: str
    description: Optional[str]
    severity: str


class DependabotAlert(TypedDict, total=False):
    """GitHub Dependabot alert object schema.

    See: https://docs.github.com/en/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository
    """

    number: int
    state: str
    dependency: DependencyDict
    security_advisory: SecurityAdvisoryDict
    security_vulnerability: Optional[SecurityVulnerabilityDict]
    url: str
    html_url: str
    created_at: Optional[str]
    updated_at: Optional[str]


class FindingCategory(str, Enum):
    """Categorization of a Dependabot security or version finding."""

    EASY_INTERNAL = "EASY_INTERNAL"
    HARD_PUBLIC_API = "HARD_PUBLIC_API"
    UNREFERENCED_TRANSITIVE = "UNREFERENCED_TRANSITIVE"


async def fetch_open_alerts(
    repo: str = REPO,
) -> AsyncIterator[list[DependabotAlert]]:
    """Step through pages of open Dependabot alerts, yielding each page.

    GitHub Dependabot alerts use cursor-based pagination via Link headers.
    See: https://docs.github.com/en/rest/using-the-rest-api/using-pagination-in-the-rest-api
    """
    endpoint: Optional[str] = (
        f"/repos/{repo}/dependabot/alerts?state=open&per_page=100"
    )
    link_next_re = re.compile(r'<([^>]+)>;\s*rel="next"')

    while endpoint:
        cmd = ["gh", "api", "--include", endpoint]
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        if proc.returncode != 0:
            print(
                f"Error fetching alerts page from {endpoint}: {stderr.decode()}",
                file=sys.stderr,
            )
            break

        split_token = b"\r\n" if b"\r\n" in stdout else b"\n"
        _, rest = stdout.split(split_token, 1)
        stream = io.BytesIO(rest)
        headers = http.client.parse_headers(stream)
        body = stream.read().decode("utf-8")

        data = json.loads(body)
        if not isinstance(data, list):
            raise ValueError(
                f"Expected JSON list from {endpoint}, got {type(data).__name__}"
            )

        page_alerts: list[DependabotAlert] = data
        yield page_alerts

        link_header = headers.get("Link")
        if link_header:
            next_match = link_next_re.search(link_header)
            if next_match:
                endpoint = next_match.group(1)
            else:
                endpoint = None
        else:
            endpoint = None


def is_internal_path(path_str: str) -> bool:
    """Check if a file path belongs strictly to internal/test/example/doc usage."""
    for substr in INTERNAL_SUBSTRINGS:
        if substr in path_str or path_str.startswith(substr):
            return True
    return False


def categorize_alert(manifest_path: str) -> FindingCategory:
    """Categorize finding based on its manifest_path."""
    if not manifest_path:
        return FindingCategory.UNREFERENCED_TRANSITIVE

    if is_internal_path(manifest_path):
        return FindingCategory.EASY_INTERNAL

    return FindingCategory.HARD_PUBLIC_API


async def load_alerts(
    args: argparse.Namespace,
) -> AsyncIterator[list[DependabotAlert]]:
    """Stream alert batches from a local JSON file or by querying the GitHub API."""
    if args.input_file:
        with open(args.input_file) as f:
            data = json.load(f)
        if not isinstance(data, list):
            raise ValueError(
                f"Expected JSON list in {args.input_file}, got {type(data).__name__}"
            )
        if data and isinstance(data[0], list):
            for batch in data:
                if not isinstance(batch, list):
                    raise ValueError(
                        f"Expected batch to be a list, got {type(batch).__name__}"
                    )
                yield batch
        else:
            yield data
    else:
        async for page in fetch_open_alerts(REPO):
            yield page


def generate_triage_report(
    alerts: list[DependabotAlert],
) -> tuple[str, dict[str, int]]:
    """Generate a structured Markdown triage report and count statistics."""
    all_items = []
    easy_list = []
    hard_list = []
    unreferenced_list = []

    for alert in alerts:
        number = alert.get("number")
        adv = alert.get("security_advisory", {})
        ghsa_id = adv.get("ghsa_id", "UNKNOWN")
        severity = adv.get("severity", "unknown").upper()
        summary = adv.get("summary", "No summary")
        html_url = alert.get("html_url", "")

        dep = alert.get("dependency", {})
        pkg = dep.get("package", {}).get("name", "unknown")
        manifest_path = dep.get("manifest_path", "")
        ecosystem = dep.get("package", {}).get("ecosystem", "pip")

        vuln = alert.get("security_vulnerability") or {}
        first_patched = vuln.get("first_patched_version") or {}
        fixed_ver = first_patched.get("identifier", "Unknown")

        cat = categorize_alert(manifest_path)

        item = {
            "number": number,
            "ghsa_id": ghsa_id,
            "severity": severity,
            "summary": summary,
            "html_url": html_url,
            "pkg": pkg,
            "manifest_path": manifest_path,
            "ecosystem": ecosystem,
            "fixed_ver": fixed_ver,
            "category": cat,
        }
        all_items.append(item)

        if cat == FindingCategory.EASY_INTERNAL:
            easy_list.append(item)
        elif cat == FindingCategory.HARD_PUBLIC_API:
            hard_list.append(item)
        else:
            unreferenced_list.append(item)

    counts = {
        "total": len(alerts),
        "easy": len(easy_list),
        "hard": len(hard_list),
        "unreferenced": len(unreferenced_list),
    }

    lines = []
    lines.append("# Dependabot Vulnerability Triage Report")
    lines.append("")
    lines.append(f"**Total Open Alerts Analyzed**: {counts['total']}")
    lines.append(
        f"- **Easy / Internal / Dev / Examples (Auto-Fixable)**: {counts['easy']}"
    )
    lines.append(f"- **Hard / Public API / Core Behavior**: {counts['hard']}")
    lines.append(f"- **Transitive / Unreferenced**: {counts['unreferenced']}")
    lines.append("")

    lines.append("## 📋 Summary: Dependabot ID to Category Mapping")
    lines.append("")
    for item in sorted(
        all_items, key=lambda x: (x["category"].value, x["number"] or 0)
    ):
        manifest_desc = (
            f"`{item['manifest_path']}`"
            if item["manifest_path"]
            else "*(no manifest)*"
        )
        lines.append(
            f"- **[Alert #{item['number']}]({item['html_url']})**: `{item['pkg']}` "
            f"({item['severity']}) -> **{item['category'].value}** "
            f"[manifest: {manifest_desc}, fix: `{item['fixed_ver']}`]"
        )
    lines.append("")

    lines.append(
        "## 🟢 Category 1: Easy / Internal Findings (Safe for Batch Auto-Fix)"
    )
    lines.append(
        "These vulnerabilities only affect internal tools, dev dependencies, tests, or examples"
    )
    lines.append(
        "(e.g., `examples/bzlmod/...`, `gazelle/examples/...`, `dev/...`). Even if Dependabot flags"
    )
    lines.append(
        "critical severity or breaking changes, they do NOT impact rules_python's public API."
    )
    lines.append(
        "**Preferred Action**: Trigger Dependabot to recreate/update PR or auto-bump & run tests."
    )
    lines.append("")
    if not easy_list:
        lines.append("*No internal easy findings found.*")
    for item in easy_list:
        lines.append(
            f"### [Alert #{item['number']}: {item['pkg']}]({item['html_url']}) ({item['severity']})"
        )
        lines.append(f"- **GHSA ID**: {item['ghsa_id']}")
        lines.append(f"- **Category**: `{item['category'].value}`")
        lines.append(f"- **Summary**: {item['summary']}")
        lines.append(f"- **Recommended Fixed Version**: `{item['fixed_ver']}`")
        lines.append(f"- **Manifest File**: `{item['manifest_path']}`")
        lines.append(
            "- **Action**: Trigger Dependabot UI / `@dependabot recreate` or auto-bump & test."
        )
        lines.append("")

    lines.append(
        "## 🔴 Category 2: Hard / Public API & Core Behavior (Requires Review)"
    )
    lines.append(
        "These vulnerabilities affect packages referenced in public toolchains or core modules."
    )
    lines.append(
        "Careful analysis is required to determine potential public API or behavior breakage."
    )
    lines.append("")
    if not hard_list:
        lines.append("*No public API hard findings found.*")
    for item in hard_list:
        lines.append(
            f"### [Alert #{item['number']}: {item['pkg']}]({item['html_url']}) ({item['severity']})"
        )
        lines.append(f"- **GHSA ID**: {item['ghsa_id']}")
        lines.append(f"- **Category**: `{item['category'].value}`")
        lines.append(f"- **Summary**: {item['summary']}")
        lines.append(f"- **Recommended Fixed Version**: `{item['fixed_ver']}`")
        lines.append(f"- **Manifest File**: `{item['manifest_path']}`")
        lines.append(
            "- **Public Impact Assessment Needed**: Evaluate if bumping changes public macro signatures or runtime behavior."
        )
        lines.append("")

    lines.append("## 🟡 Category 3: Transitive / Indirect Dependencies")
    lines.append(
        "These packages are not explicitly listed in root requirement files and are pulled in transitively."
    )
    lines.append("")
    if not unreferenced_list:
        lines.append("*No transitive alerts found.*")
    for item in unreferenced_list:
        lines.append(
            f"- **[Alert #{item['number']}: {item['pkg']}]({item['html_url']})** "
            f"({item['severity']}) -> Category: `{item['category'].value}` | Upgrade to `{item['fixed_ver']}` via parent dependency compile."
        )

    return "\n".join(lines), counts


async def notify_parent_agent(
    conversation_id: str, counts: dict[str, int], report_path: Path
) -> None:
    """Notify the parent agent of the triage results via agentapi send-message asynchronously."""
    message = (
        f"Dependabot triage complete for {REPO}.\n\n"
        f"Findings breakdown:\n"
        f"- Total open alerts: {counts['total']}\n"
        f"- Easy / Internal (Auto-Fixable): {counts['easy']}\n"
        f"- Hard / Public API (Requires Review): {counts['hard']}\n"
        f"- Transitive / Unreferenced: {counts['unreferenced']}\n\n"
        f"Report saved to: {report_path}"
    )
    cmd = [
        "agentapi",
        "send-message",
        "--title=Dependabot Triage Results",
        conversation_id,
        message,
    ]
    try:
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        _, stderr = await proc.communicate()
        if proc.returncode == 0:
            print(f"Notified parent agent (conversation {conversation_id}).")
        else:
            print(
                f"Note: Could not send agentapi message to parent agent: {stderr.decode()}",
                file=sys.stderr,
            )
    except Exception as e:
        print(
            f"Note: Could not send agentapi message to parent agent: {e}",
            file=sys.stderr,
        )


async def async_main() -> None:
    parser = argparse.ArgumentParser(description="Triage Dependabot findings.")
    parser.add_argument(
        "input_file",
        nargs="?",
        default=None,
        help="Optional local JSON file of alerts to parse instead of fetching.",
    )
    parser.add_argument(
        "--notify-conversation-id",
        default=os.environ.get(
            "PARENT_CONVERSATION_ID", os.environ.get("CONVERSATION_ID")
        ),
        help="Parent agent conversation ID to notify upon completion.",
    )
    args = parser.parse_args()

    skill_dir = Path(__file__).resolve().parents[1]
    scratch_dir = skill_dir / "scratch"
    scratch_dir.mkdir(parents=True, exist_ok=True)
    out_file = scratch_dir / "dependabot_triage_report.md"

    alerts: list[DependabotAlert] = []
    async for page in load_alerts(args):
        alerts.extend(page)

    report_md, counts = generate_triage_report(alerts)
    out_file.write_text(report_md)

    print(f"Total open alerts found: {counts['total']}")
    print(f"- Easy / Internal: {counts['easy']}")
    print(f"- Hard / Public API: {counts['hard']}")
    print(f"- Transitive / Unreferenced: {counts['unreferenced']}")
    print(f"Triage report written to {out_file}")

    if args.notify_conversation_id:
        await notify_parent_agent(args.notify_conversation_id, counts, out_file)


def main():
    asyncio.run(async_main())


if __name__ == "__main__":
    main()
