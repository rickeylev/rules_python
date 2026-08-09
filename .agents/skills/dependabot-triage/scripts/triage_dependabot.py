#!/usr/bin/env python3
"""Triage and categorize Dependabot findings in rules_python into Easy vs Hard.

Produces a categorized report mapping each Dependabot ID to its category:
- EASY_INTERNAL: Internal tools, dev dependencies, docs, tests, or examples.
- HARD_PUBLIC_API: Public APIs, core Bazel rules, or exported behaviors.
- UNREFERENCED_TRANSITIVE: Pulled in indirectly by parent dependencies.
"""

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Dict, List, Any


INTERNAL_PATH_PREFIXES = (
    "examples/",
    "tests/",
    "docs/",
    "tools/",
    "benchmarks/",
)


def fetch_open_alerts(repo: str) -> List[Dict[str, Any]]:
    """Fetch open Dependabot alerts from GitHub using gh CLI."""
    cmd = [
        "gh", "api",
        f"/repos/{repo}/dependabot/alerts?state=open&per_page=100",
        "--paginate"
    ]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return json.loads(res.stdout)
    except Exception as e:
        print(f"Error fetching alerts via gh CLI: {e}", file=sys.stderr)
        return []


def find_package_references(pkg_name: str, root_dir: Path) -> List[str]:
    """Find files referencing the package name in requirement definitions."""
    matches = []
    target_files = []
    for ext in ("requirements.in", "requirements.txt", "pyproject.toml", "MODULE.bazel", "WORKSPACE"):
        target_files.extend(root_dir.glob(f"**/{ext}"))

    pkg_lower = pkg_name.lower().replace("-", "_")
    pkg_dash = pkg_name.lower().replace("_", "-")

    for tf in target_files:
        if ".git" in tf.parts or "bazel-" in tf.name:
            continue
        try:
            content = tf.read_text(errors="ignore").lower()
            if pkg_lower in content or pkg_dash in content:
                rel_path = str(tf.relative_to(root_dir))
                matches.append(rel_path)
        except Exception:
            pass
    return matches


def categorize_alert(pkg_name: str, references: List[str]) -> str:
    """Categorize finding into EASY_INTERNAL, HARD_PUBLIC_API, or UNREFERENCED.

    Even if an advisory reports critical severity or breaking changes, if the
    package is only used internally (e.g. examples/bzlmod/...), it is EASY.
    """
    if not references:
        return "UNREFERENCED_TRANSITIVE"

    is_public = False
    for ref in references:
        is_internal = any(ref.startswith(prefix) for prefix in INTERNAL_PATH_PREFIXES)
        if not is_internal:
            is_public = True
            break

    if is_public:
        return "HARD_PUBLIC_API"
    return "EASY_INTERNAL"


def generate_triage_report(alerts: List[Dict[str, Any]], root_dir: Path) -> str:
    """Generate a structured Markdown triage report with an ID-to-Category mapping."""
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
        ecosystem = dep.get("package", {}).get("ecosystem", "pip")

        vuln = alert.get("security_vulnerability", {})
        fixed_ver = vuln.get("first_patched_version", {}).get("identifier", "Unknown")

        refs = find_package_references(pkg, root_dir)
        cat = categorize_alert(pkg, refs)

        item = {
            "number": number,
            "ghsa_id": ghsa_id,
            "severity": severity,
            "summary": summary,
            "html_url": html_url,
            "pkg": pkg,
            "ecosystem": ecosystem,
            "fixed_ver": fixed_ver,
            "refs": refs,
            "category": cat,
        }
        all_items.append(item)

        if cat == "EASY_INTERNAL":
            easy_list.append(item)
        elif cat == "HARD_PUBLIC_API":
            hard_list.append(item)
        else:
            unreferenced_list.append(item)

    lines = []
    lines.append("# Dependabot Vulnerability Triage Report")
    lines.append("")
    lines.append(f"**Total Open Alerts Analyzed**: {len(alerts)}")
    lines.append(f"- **Easy / Internal / Dev / Examples (Auto-Fixable)**: {len(easy_list)}")
    lines.append(f"- **Hard / Public API / Core Behavior**: {len(hard_list)}")
    lines.append(f"- **Transitive / Unreferenced**: {len(unreferenced_list)}")
    lines.append("")

    lines.append("## 📋 Summary: Dependabot ID to Category Mapping")
    lines.append("")
    lines.append("| Alert ID | Package | Severity | Category | Fix Version | Reference Files |")
    lines.append("|---|---|---|---|---|---|")
    for item in sorted(all_items, key=lambda x: (x["category"], x["number"] or 0)):
        ref_summary = ", ".join(item["refs"]) if item["refs"] else "*(transitive)*"
        lines.append(
            f"| [#{item['number']}]({item['html_url']}) | `{item['pkg']}` | "
            f"{item['severity']} | **{item['category']}** | `{item['fixed_ver']}` | `{ref_summary}` |"
        )
    lines.append("")

    lines.append("## 🟢 Category 1: Easy / Internal Findings (Safe for Batch Auto-Fix)")
    lines.append("These vulnerabilities only affect internal tools, dev dependencies, tests, or examples")
    lines.append("(e.g., `examples/bzlmod/...`). Even if Dependabot flags critical severity or breaking")
    lines.append("changes, they do NOT impact rules_python's public API.")
    lines.append("**Preferred Action**: Trigger Dependabot to recreate/update PR or auto-bump & run tests.")
    lines.append("")
    if not easy_list:
        lines.append("*No internal easy findings found.*")
    for item in easy_list:
        lines.append(f"### [Alert #{item['number']}: {item['pkg']}]({item['html_url']}) ({item['severity']})")
        lines.append(f"- **GHSA ID**: {item['ghsa_id']}")
        lines.append(f"- **Category**: `{item['category']}`")
        lines.append(f"- **Summary**: {item['summary']}")
        lines.append(f"- **Recommended Fixed Version**: `{item['fixed_ver']}`")
        lines.append(f"- **Referenced In**: `{', '.join(item['refs'])}`")
        lines.append("- **Action**: Trigger Dependabot UI / `@dependabot recreate` or auto-bump & test.")
        lines.append("")

    lines.append("## 🔴 Category 2: Hard / Public API & Core Behavior (Requires Review)")
    lines.append("These vulnerabilities affect packages referenced in public toolchains or core modules.")
    lines.append("Careful analysis is required to determine potential public API or behavior breakage.")
    lines.append("")
    if not hard_list:
        lines.append("*No public API hard findings found.*")
    for item in hard_list:
        lines.append(f"### [Alert #{item['number']}: {item['pkg']}]({item['html_url']}) ({item['severity']})")
        lines.append(f"- **GHSA ID**: {item['ghsa_id']}")
        lines.append(f"- **Category**: `{item['category']}`")
        lines.append(f"- **Summary**: {item['summary']}")
        lines.append(f"- **Recommended Fixed Version**: `{item['fixed_ver']}`")
        lines.append(f"- **Referenced In**: `{', '.join(item['refs'])}`")
        lines.append("- **Public Impact Assessment Needed**: Evaluate if bumping changes public macro signatures or runtime behavior.")
        lines.append("")

    lines.append("## 🟡 Category 3: Transitive / Indirect Dependencies")
    lines.append("These packages are not explicitly listed in root requirement files and are pulled in transitively.")
    lines.append("")
    if not unreferenced_list:
        lines.append("*No transitive alerts found.*")
    for item in unreferenced_list:
        lines.append(
            f"- **[Alert #{item['number']}: {item['pkg']}]({item['html_url']})** "
            f"({item['severity']}) -> Category: `{item['category']}` | Upgrade to `{item['fixed_ver']}` via parent dependency compile."
        )

    return "\n".join(lines)


def main():
    repo = os.environ.get("GITHUB_REPOSITORY", "bazel-contrib/rules_python")
    root_dir = Path(__file__).resolve().parents[3]
    if len(sys.argv) > 1:
        if sys.argv[1].endswith(".json"):
            with open(sys.argv[1]) as f:
                alerts = json.load(f)
        else:
            repo = sys.argv[1]
            alerts = fetch_open_alerts(repo)
    else:
        alerts = fetch_open_alerts(repo)

    report_md = generate_triage_report(alerts, root_dir)
    out_file = root_dir / "dependabot_triage_report.md"
    out_file.write_text(report_md)
    print(f"Triage report written to {out_file}")
    print(report_md[:1000] + "\n..." if len(report_md) > 1000 else report_md)


if __name__ == "__main__":
    main()
