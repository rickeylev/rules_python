---
name: dependabot-triage
description: Automatically fetch, triage, batch-fix internal, and assess public
  API impact for Dependabot findings in rules_python
---

Use this skill when processing large volumes of Dependabot security alerts or
version update findings in `rules_python`.

This skill automates the separation of **Easy (Internal/Dev/Examples/Docs)**
findings from **Hard (Public API/Core Behavior)** findings, produces a
categorized bullet-point mapping report for every Dependabot ID in a gitignored
scratch directory, notifies the parent agent upon completion, batch-fixes safe
internal updates, and synthesizes impact analysis for complex public alerts.

---

### Phase 1: Automated Triage & ID-to-Category Report

Run the triage engine to ingest open alerts via `gh` CLI across all pages and
classify them:

```bash
./.agents/skills/dependabot-triage/scripts/triage_dependabot.py \
  [--notify-conversation-id <parent_conversation_id>]
```

This generates `scratch/dependabot_triage_report.md` containing a master
bullet-point list mapping every **Dependabot Alert ID** to its package,
severity, fix version, and assigned category:

1. **Category 1: `EASY_INTERNAL` (Auto-Fix Candidates)**:
   * Packages referenced only in `examples/`, `tests/`, `docs/`, `dev/`, or
     `tools/` (e.g., `examples/bzlmod/...`, `gazelle/examples/...`).
   * **Crucial Classification Rule**: Even if an alert (such as alert #471) is
     flagged as Critical severity or reports "breaking changes", if the
     package is only used in examples or test requirements, it is
     `EASY_INTERNAL`. The breaking change label applies to the upstream
     library, not our public API.
2. **Category 2: `HARD_PUBLIC_API` (Requires Review)**:
   * Packages referenced in `python/`, `gazelle/`, or root public macro/module
     dependencies.
   * Requires careful impact assessment before updating.
3. **Category 3: `UNREFERENCED_TRANSITIVE`**:
   * Packages without explicit manifests, pulled in indirectly via parent
     dependency resolution.

When `--notify-conversation-id` is specified (or `PARENT_CONVERSATION_ID` is
set), `triage_dependabot.py` automatically dispatches a completion message to
the parent agent via `agentapi send-message`.

---

### Phase 2: Autonomous Batch-Fixing of Easy Internal Findings

For all Category 1 (`EASY_INTERNAL`) findings:

1. **Prefer Dependabot UI / Re-trigger**:
   * Check if Dependabot can handle the update automatically. If a PR exists
     or the Dependabot UI allows triggering a run/recreate (e.g. commenting
     `@dependabot recreate`), prefer letting Dependabot handle it!
2. **Update Source Version Constraints**: If Dependabot cannot handle it
   automatically, edit the target `requirements.in`, `pyproject.toml`, or
   `MODULE.bazel` file with the recommended fix version.
   * **Rule**: Never add Bazel copyright headers to new or modified files.
3. **Compile Locked Requirements**: Run the associated Bazel requirements
   update target to regenerate `requirements.txt`:
   ```bash
   bazel run <location>:requirements.update
   ```
4. **Run Verification Tests**:
   * Execute test suites with `bazel test --config=fast-tests //...`.
   * Always use `--config=fast-tests` for tests, but **NEVER** for non-test
     targets such as `//docs:docs` (which silently ignores non-test targets).
   * If building `//docs:docs` fails with exit code 2, try again as it is a
     known flaky build.
   * Treat any unexpected build failure or oversight as a betrayal!
5. **Draft Pull Requests**:
   * Group related internal dependency updates into coherent PRs.
   * Commit messages must be brief, explaining *why* and high-level *how*.
   * Omit `TAG` and `CONV` tags from commit messages.
   * Do not amend or rebase commits once a PR is created.

---

### Phase 3: Impact Assessment for Hard / Public API Findings

For Category 2 (`HARD_PUBLIC_API`) findings where updates may break downstream
users:

1. **Invoke Autonomous Analysis**: Delegate to a subagent via `invoke_subagent`
   to research potential breaking changes between the current and fixed
   version of the dependency.
2. **Evaluate Public Exposure**: Determine if the package change affects:
   * Public Starlark macro parameters or rule signatures.
   * Transitive dependencies exposed to users of `rules_python`.
   * Python runtime compatibility across supported interpreter versions.
3. **Generate Actionable Remediation Plan**: Produce a structured impact
   summary recommending whether to bump immediately, pin with a workaround,
   or document a deprecation notice for users.
