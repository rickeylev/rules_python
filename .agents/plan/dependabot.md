# Dependabot Processing Plan

This plan tracks tasks and updates for processing Dependabot findings and
managing security advisories in `rules_python`.

## TODO

- [x] Rename skill from `process-dependabot-findings` to `dependabot-triage`.
- [ ] Ingest all open Dependabot findings using `triage_dependabot.py`.
- [ ] Produce a structured triage report mapping each Dependabot ID to its
      assigned category (`EASY_INTERNAL`, `HARD_PUBLIC_API`, or
      `UNREFERENCED_TRANSITIVE`).
- [ ] Auto-fix internal / dev / example / doc findings in batches and run
      Bazel fast-tests.
- [ ] Analyze hard public API findings and generate impact assessments.

## High-Throughput Triage & Fix Workflow

1. **Automated Classification & ID-to-Category Report**:
   * Run `triage_dependabot.py` to classify all open Dependabot alerts into
     **Category 1 (`EASY_INTERNAL`)**, **Category 2 (`HARD_PUBLIC_API`)**, and
     **Category 3 (`UNREFERENCED_TRANSITIVE`)**.
   * Produce `dependabot_triage_report.md` containing a comprehensive summary
     table mapping each **Dependabot Alert ID** to its package, severity,
     assigned category, recommended fixed version, and referencing files.
   * **Crucial Rule**: Even if an alert (like alert #471) is marked as
     Critical severity or reports breaking changes, if the affected dependency
     is strictly in internal tests/examples (e.g., `examples/bzlmod/...`), it
     is classified as **Easy / Internal**. The breaking change applies to the
     upstream package, not `rules_python`'s public API.
2. **Dependabot Re-trigger Preference**:
   * Before manually editing files, check if the Dependabot UI allows
     triggering Dependabot to try again or generate/recreate a PR (e.g. via
     UI button or `@dependabot recreate` comment). Prefer letting Dependabot
     handle PR generation when possible.
3. **Batch Fixing Easy Findings**:
   * For internal dependencies (`examples/`, `tests/`, `docs/`, `tools/`), if
     Dependabot hasn't opened a PR, bump versions in `requirements.in` or
     `pyproject.toml`.
   * Run `bazel run <location>:requirements.update` for affected targets.
   * Run `bazel test --config=fast-tests //...` to verify zero regressions.
4. **Deep Analysis of Hard Findings**:
   * For findings affecting public APIs or core Bazel modules, perform
     impact analysis on potential breaking changes or API incompatibilities.
