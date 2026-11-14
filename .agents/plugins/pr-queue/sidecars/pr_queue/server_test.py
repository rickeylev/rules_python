#!/usr/bin/env python3
"""Unit tests for the asyncio PR Queue sidecar server."""

from __future__ import annotations

import asyncio
import json
import os
import sqlite3
import subprocess
import tempfile
import unittest
from collections.abc import Awaitable, Callable
from pathlib import Path
from unittest import mock

import fetch_pr_queue
import server

_SIDECAR_DIR = Path(__file__).resolve().parent


def _make_service(
    root: Path,
    store: server.StateStore,
    *,
    fetcher: Callable[..., Awaitable[fetch_pr_queue.PrQueueResult]] | None = None,
    conversations_db: Path | None = None,
) -> server.PrQueueService:
    return server.PrQueueService(
        store,
        worktrees_dir=root / "worktrees",
        buildbuddy_rc=root / "buildbuddy.bazelrc",
        conversations_db=(
            root / "none.db" if conversations_db is None else conversations_db
        ),
        projects_file=root / "projects.json",
        daemon_dir=root / "daemon",
        fetcher=fetcher,
    )


def _make_summary(number: int, *, head_oid: str = "abc123") -> fetch_pr_queue.PrSummary:
    return fetch_pr_queue.PrSummary(
        repo="bazel-contrib/rules_python",
        number=number,
        title=f"Fix issue #{number}",
        url=f"https://github.com/bazel-contrib/rules_python/pull/{number}",
        author="contributor",
        author_association="CONTRIBUTOR",
        is_project_member=False,
        is_bot=False,
        created_at="2026-11-10T00:00:00Z",
        updated_at="2026-11-11T00:00:00Z",
        age_days=2,
        stale_days=1,
        head_ref_name=f"feat-{number}",
        head_ref_oid=head_oid,
        base_ref_name="main",
        is_draft=False,
        review_decision="REVIEW_REQUIRED",
        review_requested_from_me=False,
        requested_reviewers=(),
        my_review_state="",
        approved_by=(),
        changes_requested_by=(),
        additions=15,
        deletions=3,
        changed_files=2,
        labels=("bug",),
        checks_state="passing",
        checks_passed=3,
        checks_failed=0,
        checks_pending=0,
        checks_total=3,
        action_needed="needs_review",
        is_stale=False,
    )


async def _send_raw_http(
    port: int, raw_request: bytes
) -> tuple[int, dict[str, str], bytes]:
    """Sends a raw HTTP/1.1 request to localhost and returns (status, headers, body)."""
    reader, writer = await asyncio.open_connection("127.0.0.1", port)
    try:
        writer.write(raw_request)
        await writer.drain()
        raw_response = await reader.read()
    finally:
        writer.close()
        await writer.wait_closed()

    head, _, body = raw_response.partition(b"\r\n\r\n")
    lines = head.decode("latin-1").split("\r\n")
    status_code = int(lines[0].split(" ", 2)[1])
    headers: dict[str, str] = {}
    for line in lines[1:]:
        if ":" in line:
            k, v = line.split(":", 1)
            headers[k.strip().lower()] = v.strip()
    return (status_code, headers, body)


async def _http_get(port: int, path: str) -> tuple[int, bytes]:
    req = (
        f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    ).encode("ascii")
    status, _headers, body = await _send_raw_http(port, req)
    return (status, body)


async def _http_post(
    port: int,
    path: str,
    payload: dict[str, object],
    *,
    header_token: str = "",
) -> tuple[int, bytes]:
    body_bytes = json.dumps(payload).encode("utf-8")
    token_hdr = f"X-Sidecar-Token: {header_token}\r\n" if header_token else ""
    req = (
        f"POST {path} HTTP/1.1\r\n"
        f"Host: 127.0.0.1:{port}\r\n"
        "Content-Type: application/json\r\n"
        f"{token_hdr}"
        f"Content-Length: {len(body_bytes)}\r\n"
        "Connection: close\r\n\r\n"
    ).encode("ascii") + body_bytes
    status, _headers, body = await _send_raw_http(port, req)
    return (status, body)


class ConfigAndStoreTest(unittest.TestCase):
    """Tests PrQueueConfig validation and StateStore persistence."""

    def test_config_validation_and_persistence(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            store = server.StateStore(Path(tmpdir))
            default_cfg = store.get_config()
            self.assertEqual(default_cfg.repo, "bazel-contrib/rules_python")
            self.assertNotIn("auto_review", default_cfg.to_dict())

            with self.assertRaises(server.PrQueueError):
                server.PrQueueConfig.from_dict({"repo": "invalid_repo_no_slash"})
            with self.assertRaises(server.PrQueueError):
                server.PrQueueConfig.from_dict({"max_age_days": "not-an-int"})
            with self.assertRaises(server.PrQueueError):
                server.PrQueueConfig.from_dict({"max_age_days": True})
            with self.assertRaises(server.PrQueueError):
                server.PrQueueConfig.from_dict({"max_age_days": -1})
            with self.assertRaises(server.PrQueueError):
                server.PrQueueConfig.from_dict({"exclude_drafts": "false"})

            updated = server.PrQueueConfig.from_dict(
                {"repo": "org/project", "max_age_days": 7, "exclude_drafts": False}
            )
            store.save_config(updated)
            reloaded = store.get_config()
            self.assertEqual(reloaded.repo, "org/project")
            self.assertEqual(reloaded.max_age_days, 7)
            self.assertFalse(reloaded.exclude_drafts)

    def test_resolve_app_and_data_dirs(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            home = root / "home"
            custom_app = root / "app"
            exec_data = root / "exec_data"
            pq_data = root / "pq_data"

            with mock.patch.dict(os.environ, {}, clear=True):
                self.assertEqual(
                    server.resolve_app_data_dir(home),
                    (home / ".gemini" / "antigravity").resolve(),
                )
                self.assertEqual(
                    server.resolve_data_dir(home),
                    (home / ".gemini" / "antigravity" / "pr_queue").resolve(),
                )

            with mock.patch.dict(
                os.environ, {"ANTIGRAVITY_APP_DATA_DIR": str(custom_app)}, clear=True
            ):
                self.assertEqual(
                    server.resolve_app_data_dir(home), custom_app.resolve()
                )
                self.assertEqual(
                    server.resolve_data_dir(home),
                    (custom_app / "pr_queue").resolve(),
                )

            with mock.patch.dict(
                os.environ,
                {
                    "ANTIGRAVITY_APP_DATA_DIR": str(custom_app),
                    "ANTIGRAVITY_EXECUTABLE_DATA_DIR": str(exec_data),
                },
                clear=True,
            ):
                self.assertEqual(server.resolve_data_dir(home), exec_data.resolve())

            with mock.patch.dict(
                os.environ,
                {
                    "ANTIGRAVITY_EXECUTABLE_DATA_DIR": str(exec_data),
                    "PR_QUEUE_DATA_DIR": str(pq_data),
                },
                clear=True,
            ):
                self.assertEqual(server.resolve_data_dir(home), pq_data.resolve())

    def test_format_conversation_title_uses_caveman_scheme(self) -> None:
        cases = [
            (
                4433,
                "feat(toolchain): allow using a py_library such as @pypi//coverage as the coverage_tool",
                "pr-4433: allow using py_library @pypi//coverage coverage_tool",
            ),
            (
                4455,
                'feat(pypi): support target_platforms = ["*"] in pip.parse',
                "pr-4455: pypi support target_platforms pip.parse",
            ),
            (
                12,
                "fix: do not drop a cached config from the file",
                "pr-12: fix not drop cached config",
            ),
            (
                19,
                "fix(bzlmod): handle the missing lockfile",
                "pr-19: fix bzlmod handle missing lockfile",
            ),
            (
                7,
                "the a an",
                "pr-7: the a an",
            ),
            (
                7,
                "",
                "pr-7: review changes",
            ),
        ]
        for pr_num, raw_title, expected in cases:
            formatted = server.format_conversation_title(pr_num, raw_title)
            self.assertEqual(formatted, expected)
            prefix, desc = formatted.split(": ", 1)
            self.assertEqual(prefix, f"pr-{pr_num}")
            self.assertLessEqual(len(desc.split()), 5)


class PrQueueServiceTest(unittest.IsolatedAsyncioTestCase):
    """Tests manual-only caching, lookup, and review launch orchestration."""

    async def test_queue_uses_cache_until_explicit_refresh(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")
            fetch_calls = 0

            async def fake_fetcher(**_kwargs: object) -> fetch_pr_queue.PrQueueResult:
                nonlocal fetch_calls
                fetch_calls += 1
                return fetch_pr_queue.PrQueueResult(
                    repo="bazel-contrib/rules_python",
                    viewer="rickeylev",
                    prs=[_make_summary(101)],
                    stale_prs=[],
                    skipped_drafts=0,
                    skipped_authored=0,
                    skipped_approved=0,
                    max_age_days=30,
                )

            svc = _make_service(root, store, fetcher=fake_fetcher)

            first = await svc.get_queue(refresh=False)
            self.assertFalse(first["from_cache"])
            self.assertEqual(fetch_calls, 1)

            second = await svc.get_queue(refresh=False)
            self.assertTrue(second["from_cache"])
            self.assertEqual(fetch_calls, 1)

            third = await svc.get_queue(refresh=True)
            self.assertFalse(third["from_cache"])
            self.assertEqual(fetch_calls, 2)

            await svc.update_config({"custom_query": "label:docs"})
            fourth = await svc.get_queue(refresh=False)
            self.assertFalse(fourth["from_cache"])
            self.assertEqual(fetch_calls, 3)

    async def test_falls_back_to_disk_cache_on_fetch_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")
            should_fail = False

            async def fake_fetcher(**_kwargs: object) -> fetch_pr_queue.PrQueueResult:
                if should_fail:
                    raise fetch_pr_queue.FetchError("network down")
                return fetch_pr_queue.PrQueueResult(
                    repo="bazel-contrib/rules_python",
                    viewer="alice",
                    prs=[_make_summary(42)],
                    stale_prs=[],
                    skipped_drafts=0,
                    skipped_authored=0,
                    skipped_approved=0,
                    max_age_days=30,
                )

            svc = _make_service(root, store, fetcher=fake_fetcher)
            await svc.get_queue(refresh=True)

            # New service instance simulates restart with empty memory cache.
            should_fail = True
            svc2 = _make_service(root, store, fetcher=fake_fetcher)
            resp = await svc2.get_queue(refresh=False)
            self.assertTrue(resp["stale_cache"])
            self.assertIn("network down", resp["error"])
            self.assertEqual(resp["prs"][0]["number"], 42)

    async def test_returns_empty_error_payload_on_cold_cache_fetch_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")

            async def failing_fetcher(
                **_kwargs: object,
            ) -> fetch_pr_queue.PrQueueResult:
                raise fetch_pr_queue.FetchError("gh auth required")

            svc = _make_service(root, store, fetcher=failing_fetcher)
            resp = await svc.get_queue(refresh=True)
            self.assertEqual(resp["repo"], "bazel-contrib/rules_python")
            self.assertEqual(resp["count"], 0)
            self.assertEqual(resp["stale_count"], 0)
            self.assertEqual(resp["prs"], [])
            self.assertEqual(resp["stale_prs"], [])
            self.assertFalse(resp["from_cache"])
            self.assertFalse(resp["stale_cache"])
            self.assertEqual(resp["fetched_at"], "")
            self.assertIn("gh auth required", resp["error"])

    async def test_lookup_pr_merges_tracked_and_sqlite_history(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")
            db_path = root / "conversation_summaries.db"
            conn = sqlite3.connect(db_path)
            conn.execute(
                "CREATE TABLE conversation_summaries "
                "(cascade_id TEXT, title TEXT, summary TEXT, updated_at INTEGER)"
            )
            conn.execute(
                "INSERT INTO conversation_summaries VALUES (?, ?, ?, ?)",
                ("cid-tracked", "pr-44: already tracked check", "", 200),
            )
            conn.execute(
                "INSERT INTO conversation_summaries VALUES (?, ?, ?, ?)",
                ("cid-caveman", "pr-44: support new parser", "", 180),
            )
            conn.execute(
                "INSERT INTO conversation_summaries VALUES (?, ?, ?, ?)",
                ("cid-old", "Earlier PR #44 check", "", 150),
            )
            conn.execute(
                "INSERT INTO conversation_summaries VALUES (?, ?, ?, ?)",
                ("cid-other", "Unrelated PR #448", "", 300),
            )
            conn.execute(
                "INSERT INTO conversation_summaries VALUES (?, ?, ?, ?)",
                ("cid-other-pr", "pr-448: unrelated parser change", "", 290),
            )
            conn.commit()
            conn.close()

            store.record_triggered_pr(
                server.TriggeredPrRecord(
                    pr_number=44,
                    repo="bazel-contrib/rules_python",
                    conversation_id="cid-tracked",
                    title="pr-44: already tracked check",
                    url="https://github.com/bazel-contrib/rules_python/pull/44",
                    head_sha="sha1",
                    head_ref="b",
                    author="bob",
                    worktree_path="/tmp/wt",
                    project_id="proj-1",
                    environment_id="env-1",
                    triggered_at=100.0,
                )
            )
            svc = _make_service(root, store, conversations_db=db_path)
            found = await svc.lookup_pr(44)
            ids = [c["conversation_id"] for c in found["conversations"]]
            self.assertEqual(ids, ["cid-tracked", "cid-caveman", "cid-old"])

    async def test_launch_review_reuses_and_forces_rerun(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")

            async def fake_fetcher(**_kw: object) -> fetch_pr_queue.PrQueueResult:
                return fetch_pr_queue.PrQueueResult(
                    repo="bazel-contrib/rules_python",
                    viewer="alice",
                    prs=[_make_summary(77, head_oid="newsha")],
                    stale_prs=[],
                    skipped_drafts=0,
                    skipped_authored=0,
                    skipped_approved=0,
                    max_age_days=30,
                )

            svc = _make_service(root, store, fetcher=fake_fetcher)
            await svc.get_queue(refresh=True)

            calls: list[dict[str, object]] = []

            async def fake_convo(**kwargs: object) -> tuple[str, str, str]:
                calls.append(dict(kwargs))
                return (f"chat-{len(calls)}", "proj-1", "env-1")

            async def fake_repo(_cfg: str, _repo: str) -> Path:
                return root / "main"

            async def fake_wt(
                _repo: Path, num: int, _slug: str, _base: str
            ) -> tuple[Path, str, str]:
                return (root / f"wt-{num}", "oldsha", "upstream")

            res1 = await svc.launch_review(
                77,
                force=False,
                repo_resolver=fake_repo,
                worktree_preparer=fake_wt,
                conversation_starter=fake_convo,
            )
            self.assertFalse(res1["reused"])
            self.assertEqual(res1["conversation_id"], "chat-1")
            self.assertEqual(res1["title"], "pr-77: fix issue #77")
            self.assertIn("/review-code", str(calls[0]["prompt"]))
            self.assertIn("upstream/main", str(calls[0]["prompt"]))

            res2 = await svc.launch_review(
                77,
                force=False,
                repo_resolver=fake_repo,
                worktree_preparer=fake_wt,
                conversation_starter=fake_convo,
            )
            self.assertTrue(res2["reused"])
            self.assertEqual(len(calls), 1)

            res3 = await svc.launch_review(
                77,
                force=True,
                repo_resolver=fake_repo,
                worktree_preparer=fake_wt,
                conversation_starter=fake_convo,
            )
            self.assertFalse(res3["reused"])
            self.assertEqual(res3["conversation_id"], "chat-2")


class AntigravityRpcAndGitTest(unittest.IsolatedAsyncioTestCase):
    """Tests LS RPC integration, worktree creation, and remote detection."""

    async def test_ensure_project_environment_creates_missing_env(self) -> None:
        calls: list[tuple[str, dict[str, object]]] = []
        main_repo = Path("/tmp/main-repo").resolve()
        wt_path = Path("/tmp/worktrees/review-pr-5").resolve()

        async def fake_rpc(
            _ls: object,
            method: str,
            body: dict[str, object],
        ) -> dict[str, object]:
            calls.append((method, body))
            if method == "ListProjects":
                return {"projectIds": ["proj-archived", "proj-42"]}
            if method == "ReadProjects":
                return {
                    "projects": [
                        {
                            "id": "proj-archived",
                            "archived": True,
                            "projectResources": {
                                "resources": [
                                    {"gitFolder": {"folderUri": f"file://{main_repo}"}}
                                ]
                            },
                        },
                        {
                            "id": "proj-42",
                            "name": "main-repo",
                            "projectResources": {
                                "resources": [
                                    {"gitFolder": {"folderUri": f"file://{main_repo}"}}
                                ]
                            },
                            "environments": {"environments": []},
                        },
                    ]
                }
            if method == "AddEnvironmentToProject":
                env = body["environment"]
                assert isinstance(env, dict)  # type assert
                return {"environment": {"id": env["id"]}}
            return {}

        proj_id, env_id = await server.ensure_project_environment(
            {"port": 1, "csrf_token": "t"},
            main_repo,
            wt_path,
            ls_caller=fake_rpc,
        )
        self.assertEqual(proj_id, "proj-42")
        self.assertTrue(env_id)
        self.assertEqual(
            [m for m, _ in calls],
            ["ListProjects", "ReadProjects", "AddEnvironmentToProject"],
        )
        self.assertEqual(calls[1][1], {"ids": ["proj-archived", "proj-42"]})

    async def test_ensure_project_environment_fails_without_matching_project(
        self,
    ) -> None:
        async def empty_rpc(
            _ls: object,
            method: str,
            _body: dict[str, object],
        ) -> dict[str, object]:
            if method == "ListProjects":
                return {"projectIds": []}
            return {}

        with self.assertRaises(server.PrQueueError):
            await server.ensure_project_environment(
                {"port": 1, "csrf_token": "t"},
                Path("/tmp/unregistered-repo"),
                Path("/tmp/wt"),
                ls_caller=empty_rpc,
            )

    async def test_ensure_project_environment_uses_source_conversation_fallback(
        self,
    ) -> None:
        calls: list[tuple[str, dict[str, object]]] = []
        wt_path = Path("/tmp/wt-pr-99").resolve()

        async def fake_rpc(
            _ls: object,
            method: str,
            body: dict[str, object],
        ) -> dict[str, object]:
            calls.append((method, body))
            if method == "ListProjects":
                return {"projectIds": ["proj-pane"]}
            if method == "ReadProjects":
                return {
                    "projects": [
                        {
                            "id": "proj-pane",
                            "name": "unrelated-worktree-name",
                            "projectResources": {
                                "resources": [
                                    {"folderUri": "file:///tmp/other-worktree"}
                                ]
                            },
                            "environments": {"environments": []},
                        }
                    ]
                }
            if method == "GetConversationMetadata":
                if body.get("conversationId") == "chat-outside":
                    return {"metadata": {"projectId": "outside-of-project"}}
                return {"metadata": {"projectId": "proj-pane"}}
            if method == "AddEnvironmentToProject":
                return {"environment": {"id": "env-new"}}
            return {}

        proj_id, env_id = await server.ensure_project_environment(
            {"port": 1, "csrf_token": "t"},
            Path("/tmp/unmatched-repo").resolve(),
            wt_path,
            source_conversation_id="chat-in-pane",
            ls_caller=fake_rpc,
        )
        self.assertEqual((proj_id, env_id), ("proj-pane", "env-new"))
        self.assertEqual(
            calls[2], ("GetConversationMetadata", {"conversationId": "chat-in-pane"})
        )

        with self.assertRaises(server.PrQueueError):
            await server.ensure_project_environment(
                {"port": 1, "csrf_token": "t"},
                Path("/tmp/unmatched-repo").resolve(),
                wt_path,
                source_conversation_id="chat-outside",
                ls_caller=fake_rpc,
            )

    async def test_resolve_default_model_prefers_config_and_falls_back(
        self,
    ) -> None:
        async def cascade_cfg_rpc(
            _ls: object,
            method: str,
            _body: dict[str, object],
        ) -> dict[str, object]:
            if method == "GetCascadeModelConfigData":
                return {
                    "defaultOverrideModelConfig": {
                        "modelOrAlias": {"model": "MODEL_OVERRIDE"}
                    }
                }
            raise AssertionError(f"Unexpected RPC {method}")

        self.assertEqual(
            await server.resolve_default_model(
                {"port": 1, "csrf_token": "t"}, ls_caller=cascade_cfg_rpc
            ),
            "MODEL_OVERRIDE",
        )

        async def client_cfg_rpc(
            _ls: object,
            method: str,
            _body: dict[str, object],
        ) -> dict[str, object]:
            if method == "GetCascadeModelConfigData":
                return {
                    "clientModelConfigs": [
                        {"disabled": True, "modelOrAlias": {"model": "MODEL_SKIP"}},
                        {"modelOrAlias": {"model": "MODEL_CLIENT"}},
                    ]
                }
            return {}

        self.assertEqual(
            await server.resolve_default_model(
                {"port": 1, "csrf_token": "t"}, ls_caller=client_cfg_rpc
            ),
            "MODEL_CLIENT",
        )

        async def avail_fallback_rpc(
            _ls: object,
            method: str,
            _body: dict[str, object],
        ) -> dict[str, object]:
            if method == "GetCascadeModelConfigData":
                raise server.PrQueueError("unsupported")
            if method == "GetAvailableModels":
                return {
                    "response": {
                        "defaultAgentModelId": "auto",
                        "models": {"auto": {"model": "MODEL_AUTO", "disabled": False}},
                        "tieredModelIds": {"pro": ["snowball-subagent"]},
                    }
                }
            return {}

        self.assertEqual(
            await server.resolve_default_model(
                {"port": 1, "csrf_token": "t"}, ls_caller=avail_fallback_rpc
            ),
            "MODEL_AUTO",
        )

        async def empty_models_rpc(
            _ls: object,
            _method: str,
            _body: dict[str, object],
        ) -> dict[str, object]:
            return {}

        with self.assertRaises(server.PrQueueError):
            await server.resolve_default_model(
                {"port": 1, "csrf_token": "t"}, ls_caller=empty_models_rpc
            )

    async def test_start_pr_review_conversation_sends_title_and_message(self) -> None:
        calls: list[tuple[str, dict[str, object]]] = []
        main_repo = Path("/tmp/repo").resolve()
        wt_path = Path("/tmp/wt").resolve()

        async def fake_rpc(
            _ls: object,
            method: str,
            body: dict[str, object],
        ) -> dict[str, object]:
            calls.append((method, body))
            if method == "ListProjects":
                return {"projectIds": ["proj-9"]}
            if method == "ReadProjects":
                return {
                    "projects": [
                        {
                            "id": "proj-9",
                            "name": "repo",
                            "projectResources": {
                                "resources": [
                                    {"gitFolder": {"folderUri": f"file://{main_repo}"}}
                                ]
                            },
                            "environments": {
                                "environments": [
                                    {
                                        "id": "env-wt",
                                        "name": wt_path.name,
                                        "resources": {
                                            "resources": [
                                                {"folderUri": f"file://{wt_path}"}
                                            ]
                                        },
                                    }
                                ]
                            },
                        }
                    ]
                }
            if method == "GetCascadeModelConfigData":
                return {
                    "defaultOverrideModelConfig": {
                        "modelOrAlias": {"model": "MODEL_PRO"}
                    }
                }
            if method == "StartCascade":
                return {"cascadeId": "new-chat-uuid"}
            return {}

        cid, pid, eid = await server.start_pr_review_conversation(
            main_repo=main_repo,
            worktree_path=wt_path,
            prompt="Review PR #5",
            title="pr-5: add test check",
            ls_caller=fake_rpc,
            ls_resolver=lambda: {"scheme": "http", "port": 1234, "csrf_token": "abc"},
        )
        self.assertEqual((cid, pid, eid), ("new-chat-uuid", "proj-9", "env-wt"))
        methods = [m for m, _ in calls]
        self.assertEqual(
            methods,
            [
                "ListProjects",
                "ReadProjects",
                "GetCascadeModelConfigData",
                "StartCascade",
                "UpdateConversationAnnotations",
                "SendUserCascadeMessage",
            ],
        )
        start_req = calls[3][1]
        self.assertEqual(
            start_req["projectEnvConfig"],
            {"projectId": "proj-9", "environmentId": "env-wt"},
        )
        self.assertNotIn("workspaceUris", start_req)
        self.assertEqual(start_req["requestedModel"], "MODEL_PRO")
        self.assertEqual(
            calls[4][1]["annotations"],
            {"title": "pr-5: add test check"},
        )
        self.assertEqual(
            calls[5][1]["cascadeConfig"],
            {
                "plannerConfig": {
                    "planModel": "MODEL_PRO",
                    "requestedModel": {"model": "MODEL_PRO"},
                }
            },
        )

    async def test_prepare_pr_worktree_with_real_temp_git_repo(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            remote_repo = tmp / "remote.git"
            main_repo = tmp / "rules_python"
            worktrees_dir = tmp / "worktrees"
            buildbuddy_rc = tmp / "buildbuddy.bazelrc"
            github_url = "https://github.com/bazel-contrib/rules_python.git"

            def _setup_repo() -> str:
                subprocess.run(["git", "init", "--bare", str(remote_repo)], check=True)
                subprocess.run(["git", "init", str(main_repo)], check=True)
                subprocess.run(
                    [
                        "git",
                        "-C",
                        str(main_repo),
                        "config",
                        "user.email",
                        "t@example.com",
                    ],
                    check=True,
                )
                subprocess.run(
                    ["git", "-C", str(main_repo), "config", "user.name", "Tester"],
                    check=True,
                )
                subprocess.run(
                    [
                        "git",
                        "-C",
                        str(main_repo),
                        "config",
                        f"url.{remote_repo}.insteadOf",
                        github_url,
                    ],
                    check=True,
                )
                (main_repo / "README.md").write_text("base\n", encoding="utf-8")
                subprocess.run(["git", "-C", str(main_repo), "add", "."], check=True)
                subprocess.run(
                    ["git", "-C", str(main_repo), "commit", "-m", "init"],
                    check=True,
                )
                subprocess.run(
                    ["git", "-C", str(main_repo), "branch", "-M", "main"], check=True
                )
                subprocess.run(
                    [
                        "git",
                        "-C",
                        str(main_repo),
                        "remote",
                        "add",
                        "upstream",
                        github_url,
                    ],
                    check=True,
                )
                subprocess.run(
                    ["git", "-C", str(main_repo), "push", str(remote_repo), "main"],
                    check=True,
                )
                # Create a second commit to simulate PR #12 head ref on upstream.
                subprocess.run(
                    ["git", "-C", str(main_repo), "checkout", "-b", "pr-branch"],
                    check=True,
                )
                (main_repo / "change.txt").write_text("pr change\n", encoding="utf-8")
                subprocess.run(["git", "-C", str(main_repo), "add", "."], check=True)
                subprocess.run(
                    ["git", "-C", str(main_repo), "commit", "-m", "pr commit"],
                    check=True,
                )
                sha = subprocess.check_output(
                    ["git", "-C", str(main_repo), "rev-parse", "HEAD"], text=True
                ).strip()
                subprocess.run(
                    [
                        "git",
                        "-C",
                        str(main_repo),
                        "push",
                        str(remote_repo),
                        f"{sha}:refs/pull/12/head",
                    ],
                    check=True,
                )
                subprocess.run(
                    ["git", "-C", str(main_repo), "checkout", "main"], check=True
                )
                return sha

            pr_sha = await asyncio.to_thread(_setup_repo)

            wt_path, checked_sha, remote_name = await server.prepare_pr_worktree(
                main_repo,
                12,
                "bazel-contrib/rules_python",
                "main",
                worktrees_dir=worktrees_dir,
                buildbuddy_rc=buildbuddy_rc,
            )
            self.assertEqual(remote_name, "upstream")
            self.assertEqual(checked_sha, pr_sha)
            self.assertTrue((wt_path / "change.txt").is_file())

    def test_remote_matches_repo_rejects_domain_spoofing(self) -> None:
        repo = "bazel-contrib/rules_python"
        self.assertTrue(
            server._remote_matches_repo(
                "https://github.com/bazel-contrib/rules_python.git", repo
            )
        )
        self.assertTrue(
            server._remote_matches_repo(
                "git@github.com:bazel-contrib/rules_python.git", repo
            )
        )
        self.assertFalse(
            server._remote_matches_repo(
                "https://evilgithub.com/bazel-contrib/rules_python.git", repo
            )
        )
        self.assertFalse(
            server._remote_matches_repo(
                "git@github.com.evil.com:bazel-contrib/rules_python.git", repo
            )
        )
        self.assertFalse(
            server._remote_matches_repo(
                "file:///tmp/github.com/bazel-contrib/rules_python.git", repo
            )
        )

    def test_unverified_tls_rejects_non_loopback_host(self) -> None:
        with self.assertRaises(server.PrQueueError):
            server._call_ls_rpc_sync(
                {
                    "scheme": "https",
                    "host": "evil.example.com",
                    "port": 443,
                    "csrf_token": "tok",
                },
                "ReadProjects",
                {},
            )

    async def test_detect_remote_and_resolve_base_repo_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            repo_dir = tmp / "other"

            def _init_other() -> None:
                subprocess.run(["git", "init", str(repo_dir)], check=True)
                subprocess.run(
                    [
                        "git",
                        "-C",
                        str(repo_dir),
                        "remote",
                        "add",
                        "origin",
                        "https://github.com/other/project.git",
                    ],
                    check=True,
                )

            await asyncio.to_thread(_init_other)
            with self.assertRaises(server.PrQueueError):
                await server.detect_remote_for_repo(
                    repo_dir, "bazel-contrib/rules_python"
                )
            with self.assertRaises(server.PrQueueError):
                await server.resolve_base_repo(
                    "",
                    "nonexistent/repo",
                    projects_file=tmp / "missing_projects.json",
                    daemon_dir=tmp / "missing_daemon",
                )


class HttpServerTest(unittest.IsolatedAsyncioTestCase):
    """Tests asyncio HTTP routes, concurrency, chunked POSTs, and token checks."""

    async def test_http_endpoints_and_token_enforcement(self) -> None:
        mock_token = "mock-sidecar-check-value"
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")
            calls = 0

            async def fake_fetcher(**_kwargs: object) -> fetch_pr_queue.PrQueueResult:
                nonlocal calls
                calls += 1
                return fetch_pr_queue.PrQueueResult(
                    repo="bazel-contrib/rules_python",
                    viewer="alice",
                    prs=[_make_summary(55)],
                    stale_prs=[],
                    skipped_drafts=0,
                    skipped_authored=0,
                    skipped_approved=0,
                    max_age_days=30,
                )

            svc = _make_service(root, store, fetcher=fake_fetcher)
            with self.assertRaises(server.PrQueueError):
                server.PrQueueHttpServer(
                    ("127.0.0.1", 0),
                    svc,
                    html_path=_SIDECAR_DIR / "index.html",
                    token="",
                    preload_candidates=(),
                )
            httpd = server.PrQueueHttpServer(
                ("127.0.0.1", 0),
                svc,
                html_path=_SIDECAR_DIR / "index.html",
                token=mock_token,
                preload_candidates=(root / "missing-preload.js",),
            )
            port = await httpd.start()
            try:
                status, html_bytes = await _http_get(port, "/")
                self.assertEqual(status, 200)
                html_body = html_bytes.decode("utf-8")
                self.assertIn("/preload.js", html_body)
                self.assertIn("Project member", html_body)
                self.assertNotIn("· requested:", html_body)
                self.assertNotIn("setInterval", html_body)

                status_preload, _ = await _http_get(port, "/preload.js")
                self.assertEqual(status_preload, 404)

                status_bad_refresh, _ = await _http_get(port, "/api/queue?refresh=bad")
                self.assertEqual(status_bad_refresh, 400)

                status_q1, body_q1 = await _http_get(port, "/api/queue?refresh=0")
                self.assertEqual(status_q1, 200)
                q1 = json.loads(body_q1.decode("utf-8"))
                self.assertEqual(q1["count"], 1)

                status_q2, body_q2 = await _http_get(port, "/api/queue?refresh=0")
                self.assertEqual(status_q2, 200)
                q2 = json.loads(body_q2.decode("utf-8"))
                self.assertTrue(q2["from_cache"])
                self.assertEqual(calls, 1)

                # POST without token must return 403.
                status_no_tok, _ = await _http_post(port, "/api/config", {})
                self.assertEqual(status_no_tok, 403)

                # POST with valid token succeeds.
                status_tok, body_tok = await _http_post(
                    port,
                    "/api/config",
                    {"custom_query": "is:open"},
                    header_token=mock_token,
                )
                self.assertEqual(status_tok, 200)
                cfg_resp = json.loads(body_tok.decode("utf-8"))
                self.assertTrue(cfg_resp["ok"])
                self.assertEqual(cfg_resp["config"]["custom_query"], "is:open")

                # Chunked POST request succeeds.
                chunk_payload = json.dumps({"custom_query": "is:draft"}).encode("utf-8")
                chunked_req = (
                    (
                        f"POST /api/config HTTP/1.1\r\n"
                        f"Host: 127.0.0.1:{port}\r\n"
                        "Content-Type: application/json\r\n"
                        f"X-Sidecar-Token: {mock_token}\r\n"
                        "Transfer-Encoding: chunked\r\n"
                        "Connection: close\r\n\r\n"
                        f"{len(chunk_payload):x}\r\n"
                    ).encode("ascii")
                    + chunk_payload
                    + b"\r\n0\r\n\r\n"
                )
                status_chunk, _hdr, body_chunk = await _send_raw_http(port, chunked_req)
                self.assertEqual(status_chunk, 200)
                self.assertEqual(
                    json.loads(body_chunk.decode("utf-8"))["config"]["custom_query"],
                    "is:draft",
                )
            finally:
                await httpd.close()

    async def test_serves_healthz_concurrently_during_slow_fetch(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")
            started = asyncio.Event()
            release = asyncio.Event()

            async def slow_fetcher(**_kwargs: object) -> fetch_pr_queue.PrQueueResult:
                started.set()
                await release.wait()
                return fetch_pr_queue.PrQueueResult(
                    repo="bazel-contrib/rules_python",
                    viewer="alice",
                    prs=[_make_summary(88)],
                    stale_prs=[],
                    skipped_drafts=0,
                    skipped_authored=0,
                    skipped_approved=0,
                    max_age_days=30,
                )

            svc = _make_service(root, store, fetcher=slow_fetcher)
            httpd = server.PrQueueHttpServer(
                ("127.0.0.1", 0),
                svc,
                html_path=_SIDECAR_DIR / "index.html",
                token="",
                preload_candidates=(),
                allow_unauthenticated=True,
            )
            port = await httpd.start()
            try:
                queue_task = asyncio.create_task(
                    _http_get(port, "/api/queue?refresh=1")
                )
                await asyncio.wait_for(started.wait(), timeout=2.0)

                # While slow_fetcher is suspended, /healthz completes immediately.
                health_status, health_body = await asyncio.wait_for(
                    _http_get(port, "/healthz"), timeout=2.0
                )
                self.assertEqual(health_status, 200)
                self.assertTrue(json.loads(health_body.decode("utf-8"))["ok"])

                release.set()
                q_status, q_body = await asyncio.wait_for(queue_task, timeout=2.0)
                self.assertEqual(q_status, 200)
                self.assertEqual(
                    json.loads(q_body.decode("utf-8"))["prs"][0]["number"], 88
                )

                # Reject boolean pr_number, non-bool force, bad source_conversation_id,
                # and oversized payload.
                status_bool, _ = await _http_post(
                    port, "/api/review", {"pr_number": True}
                )
                self.assertEqual(status_bool, 400)
                status_force, _ = await _http_post(
                    port, "/api/review", {"pr_number": 5, "force": "false"}
                )
                self.assertEqual(status_force, 400)
                status_src, _ = await _http_post(
                    port,
                    "/api/review",
                    {"pr_number": 5, "source_conversation_id": 123},
                )
                self.assertEqual(status_src, 400)

                oversized_req = (
                    f"POST /api/config HTTP/1.1\r\n"
                    f"Host: 127.0.0.1:{port}\r\n"
                    "Content-Type: application/json\r\n"
                    f"Content-Length: {server._MAX_POST_BODY_BYTES + 1}\r\n"
                    "Connection: close\r\n\r\n"
                ).encode("ascii")
                status_big, _hdr, _body = await _send_raw_http(port, oversized_req)
                self.assertEqual(status_big, 413)
            finally:
                await httpd.close()

    async def test_concurrent_launch_review_shares_per_pr_lock(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")

            async def fake_fetcher(**_kw: object) -> fetch_pr_queue.PrQueueResult:
                return fetch_pr_queue.PrQueueResult(
                    repo="bazel-contrib/rules_python",
                    viewer="alice",
                    prs=[_make_summary(91)],
                    stale_prs=[],
                    skipped_drafts=0,
                    skipped_authored=0,
                    skipped_approved=0,
                    max_age_days=30,
                )

            svc = _make_service(root, store, fetcher=fake_fetcher)
            await svc.get_queue(refresh=True)

            entered = asyncio.Event()
            release = asyncio.Event()
            launch_count = 0

            async def slow_convo(**_kw: object) -> tuple[str, str, str]:
                nonlocal launch_count
                launch_count += 1
                entered.set()
                await release.wait()
                return ("chat-single", "proj-1", "env-1")

            async def fake_repo(_cfg: str, _repo: str) -> Path:
                return root / "main"

            async def fake_wt(
                _repo: Path, num: int, _slug: str, _base: str
            ) -> tuple[Path, str, str]:
                return (root / f"wt-{num}", "sha1", "upstream")

            t1 = asyncio.create_task(
                svc.launch_review(
                    91,
                    repo_resolver=fake_repo,
                    worktree_preparer=fake_wt,
                    conversation_starter=slow_convo,
                )
            )
            await asyncio.wait_for(entered.wait(), timeout=2.0)
            t2 = asyncio.create_task(
                svc.launch_review(
                    91,
                    repo_resolver=fake_repo,
                    worktree_preparer=fake_wt,
                    conversation_starter=slow_convo,
                )
            )
            release.set()
            res1, res2 = await asyncio.wait_for(asyncio.gather(t1, t2), timeout=2.0)
            self.assertEqual(launch_count, 1)
            self.assertEqual(res1["conversation_id"], "chat-single")
            self.assertEqual(res2["conversation_id"], "chat-single")
            self.assertFalse(res1["reused"])
            self.assertTrue(res2["reused"])

    async def test_retries_only_idempotent_ls_rpcs(self) -> None:
        attempts = 0

        def flaky_sync(_ls: object, _method: str, _body: object) -> dict[str, object]:
            nonlocal attempts
            attempts += 1
            if attempts < 2:
                raise server._TransientLsRpcError("503 Service Unavailable")
            return {"ok": True}

        with mock.patch.object(
            server, "_call_ls_rpc_sync", side_effect=flaky_sync
        ), mock.patch.object(fetch_pr_queue.asyncio, "sleep", return_value=None):
            res = await server.call_ls_rpc(
                {"port": 1, "csrf_token": "t"}, "ReadProjects", {}, retryable=True
            )
        self.assertEqual(res, {"ok": True})
        self.assertEqual(attempts, 2)

        attempts = 0
        with mock.patch.object(
            server, "_call_ls_rpc_sync", side_effect=flaky_sync
        ), mock.patch.object(
            fetch_pr_queue.asyncio, "sleep", return_value=None
        ), self.assertRaises(server._TransientLsRpcError):
            await server.call_ls_rpc(
                {"port": 1, "csrf_token": "t"}, "StartCascade", {}, retryable=False
            )
        self.assertEqual(attempts, 1)

    async def test_http_review_forwards_source_conversation_id(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            store = server.StateStore(root / "state")
            seen_kwargs: dict[str, object] = {}

            async def fake_fetcher(**_kw: object) -> fetch_pr_queue.PrQueueResult:
                return fetch_pr_queue.PrQueueResult(
                    repo="bazel-contrib/rules_python",
                    viewer="alice",
                    prs=[_make_summary(92)],
                    stale_prs=[],
                    skipped_drafts=0,
                    skipped_authored=0,
                    skipped_approved=0,
                    max_age_days=30,
                )

            async def fake_repo(_cfg: str, _repo: str, **_kw: object) -> Path:
                return root / "main"

            async def fake_wt(
                _repo: Path, num: int, _slug: str, _base: str, **_kw: object
            ) -> tuple[Path, str, str]:
                return (root / f"wt-{num}", "sha2", "upstream")

            async def fake_start_convo(**kwargs: object) -> tuple[str, str, str]:
                seen_kwargs.update(kwargs)
                return ("chat-92", "proj-pane", "env-92")

            svc = _make_service(root, store, fetcher=fake_fetcher)
            await svc.get_queue(refresh=True)
            httpd = server.PrQueueHttpServer(
                ("127.0.0.1", 0),
                svc,
                html_path=_SIDECAR_DIR / "index.html",
                token="",
                preload_candidates=(),
                allow_unauthenticated=True,
            )
            port = await httpd.start()
            try:
                with mock.patch.object(
                    server, "resolve_base_repo", side_effect=fake_repo
                ), mock.patch.object(
                    server, "prepare_pr_worktree", side_effect=fake_wt
                ), mock.patch.object(
                    server, "start_pr_review_conversation", side_effect=fake_start_convo
                ):
                    status, raw_body = await _http_post(
                        port,
                        "/api/review",
                        {
                            "pr_number": 92,
                            "source_conversation_id": "  pane-conv-7  ",
                        },
                    )
            finally:
                await httpd.close()

            self.assertEqual(status, 200)
            payload = json.loads(raw_body.decode("utf-8"))
            self.assertEqual(payload["conversation_id"], "chat-92")
            self.assertEqual(seen_kwargs.get("source_conversation_id"), "pane-conv-7")


if __name__ == "__main__":
    unittest.main()
