#!/usr/bin/env python3
"""Asyncio HTTP sidecar server for the PR Queue Antigravity plugin.

Serves an adaptive dark/light side panel UI (`index.html`), proxies Antigravity
`preload.js`, caches fetched GitHub PR queues on demand (with no automatic
reviews or background refreshes), resolves matching PR review chats, and
provisions isolated Git worktrees to launch `/review-code` conversations.

Usage:
  python3 .agents/plugins/pr-queue/sidecars/pr_queue/server.py [--port 8765]

Environment variables:
  ANTIGRAVITY_SIDECAR_WEB_PORT: Listening port injected by Antigravity when
    `--port` is omitted (`PORT` is also honored for standalone local dev).
  ANTIGRAVITY_SIDECAR_UI_TOKEN: Shared token expected in the `X-Sidecar-Token`
    header on POST requests.
  ANTIGRAVITY_LS_ADDRESS / ANTIGRAVITY_CSRF_TOKEN: Antigravity language server
    RPC endpoint and CSRF token (falls back to `daemon/*.json`).
  ANTIGRAVITY_APP_DATA_DIR: Antigravity app data root containing `daemon`,
    `worktrees`, `lib`, and `conversation_summaries.db` (default:
    `~/.gemini/antigravity`).
  PR_QUEUE_DATA_DIR / ANTIGRAVITY_EXECUTABLE_DATA_DIR: Override state
    persistence directory (default: `<ANTIGRAVITY_APP_DATA_DIR>/pr_queue`).
"""

from __future__ import annotations

import argparse
import asyncio
import contextlib
import dataclasses
import datetime
import functools
import hmac
import json
import logging
import os
import re
import signal
import sqlite3
import ssl
import sys
import threading
import time
import uuid
from collections.abc import Awaitable, Callable, Sequence
from http import HTTPStatus
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, unquote, urlparse, urlsplit
from urllib.request import Request, urlopen

from fetch_pr_queue import (
    DEFAULT_MAX_AGE_DAYS,
    DEFAULT_REPO,
    PR_JSON_FIELDS,
    FetchError,
    PrQueueResult,
    fetch_pr_queue_async,
    format_subprocess_error,
    get_current_gh_user_async,
    is_transient_error,
    normalize_pr,
    retry_async,
    run_gh_command_async,
    run_subprocess_async,
)

# =============================================================================
# 1. Global Constants
# =============================================================================

DEFAULT_HOST = "127.0.0.1"
_SIDECAR_DIR = Path(__file__).resolve().parent
# Git fetch over HTTPS/SSH can take noticeable time on cold refs.
_GIT_FETCH_TIMEOUT_SECONDS = 60
# Local git inspection and worktree creation are normally fast.
_GIT_LOCAL_TIMEOUT_SECONDS = 30
# Localhost Antigravity language server RPCs normally complete within ms.
_LS_RPC_TIMEOUT_SECONDS = 5
# Bound SQLite lock waits when Antigravity writes conversation summaries.
_SQLITE_BUSY_TIMEOUT_MS = 2000
# Fetch enough LIKE matches that exact token filtering avoids false negatives.
_SQLITE_FILTER_CANDIDATE_LIMIT = 50
# Guard client HTTP header/body reads against stalled connections.
_HTTP_IO_TIMEOUT_SECONDS = 30
# Cap request headers to mitigate header-flood payloads against the local server.
_MAX_HEADER_LINES = 100
_MAX_HEADER_BYTES = 64 * 1024
# Cap POST request bodies at 1 MiB to avoid unbounded buffering in memory.
_MAX_POST_BODY_BYTES = 1024 * 1024
# Keep spawned sidebar conversation titles concise (`pr-<N>: <5 words>`).
_MAX_CONVERSATION_TITLE_CHARS = 120
_MAX_CONVERSATION_TITLE_WORDS = 5
_TITLE_PUNCT_STRIP_CHARS = ".,:;!?()[]{}\"'`~*"
_HAS_ALNUM_RE = re.compile(r"[A-Za-z0-9]")
_BRACKET_PREFIX_RE = re.compile(r"^(?:\[[^\]]+\]\s*)+")
_CONVENTIONAL_COMMIT_RE = re.compile(
    r"^\s*(?P<ctype>[A-Za-z]+)(?:\((?P<scope>[^)]+)\))?!?:\s*(?P<rest>.+)$"
)
_DROP_CONVENTIONAL_TYPES = frozenset(
    {
        "build",
        "chore",
        "ci",
        "feat",
        "perf",
        "refactor",
        "style",
    }
)
_CAVEMAN_STOPWORDS = frozenset(
    {
        "a",
        "about",
        "also",
        "am",
        "an",
        "and",
        "are",
        "as",
        "at",
        "be",
        "been",
        "being",
        "by",
        "can",
        "could",
        "did",
        "do",
        "does",
        "for",
        "from",
        "had",
        "has",
        "have",
        "here",
        "in",
        "into",
        "is",
        "it",
        "its",
        "just",
        "may",
        "might",
        "my",
        "of",
        "on",
        "only",
        "or",
        "our",
        "shall",
        "should",
        "so",
        "some",
        "such",
        "than",
        "that",
        "the",
        "their",
        "then",
        "there",
        "these",
        "this",
        "those",
        "through",
        "to",
        "via",
        "was",
        "were",
        "when",
        "where",
        "which",
        "while",
        "who",
        "whom",
        "whose",
        "will",
        "with",
        "would",
        "your",
    }
)
# Single-PR `gh pr view` lookups fetch one PR without pagination.
_SINGLE_PR_VIEW_TIMEOUT_SECONDS = 15

_TRANSIENT_LS_HTTP_CODES = frozenset(
    {
        # keep-sorted start
        500,
        502,
        503,
        504,
        # keep-sorted end
    }
)

_DEFAULT_APP_SUBDIR = Path(".gemini") / "antigravity"
_DEFAULT_PROJECTS_JSON = Path(".gemini") / "projects.json"
_DEFAULT_BUILDBUDDY_RC = Path(".config") / "rules_python" / "buildbuddy.bazelrc"

_LOOPBACK_HOSTS = frozenset({"127.0.0.1", "localhost", "::1"})
_REVIEW_SKILL_REL_PATH = ".agents/skills/review-code/SKILL.md"
_VALID_REPO_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
_LOGGER = logging.getLogger("pr_queue.server")


# =============================================================================
# 2. CLI Arguments
# =============================================================================


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    """Parses CLI flags for the PR Queue sidecar HTTP server."""
    # Prefer Antigravity's web port env var; allow PORT for standalone local dev.
    env_port = (
        os.environ.get("ANTIGRAVITY_SIDECAR_WEB_PORT") or os.environ.get("PORT") or "0"
    )
    try:
        default_port = int(env_port)
    except ValueError:
        default_port = 0

    parser = argparse.ArgumentParser(
        description="Run the PR Queue sidecar asyncio HTTP server."
    )
    parser.add_argument(
        "--host",
        default=DEFAULT_HOST,
        help="Interface to bind the HTTP server to (default: %(default)s)",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=default_port,
        help="TCP port to listen on; 0 selects an ephemeral port",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate state directory and config without starting HTTP loop",
    )
    parser.add_argument(
        "--allow-unauthenticated",
        action="store_true",
        help=(
            "Allow POST requests without X-Sidecar-Token for standalone dev "
            "(AGENTS: requires user confirmation to use)"
        ),
    )
    args = parser.parse_args(argv)
    if args.port < 0 or args.port > 65535:
        parser.error("--port must be between 0 and 65535.")
    return args


# =============================================================================
# 3. Helper Functions
# =============================================================================


def _configure_logging() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        stream=sys.stderr,
    )


def resolve_app_data_dir(home_dir: Path | None = None) -> Path:
    """Returns the Antigravity app data directory, honoring ANTIGRAVITY_APP_DATA_DIR."""
    override = os.environ.get("ANTIGRAVITY_APP_DATA_DIR", "").strip()
    if override:
        return Path(override).expanduser().resolve()
    base = Path.home() if home_dir is None else home_dir
    return (base / _DEFAULT_APP_SUBDIR).resolve()


def resolve_data_dir(home_dir: Path | None = None) -> Path:
    """Returns persistence dir, honoring PR_QUEUE_DATA_DIR and Antigravity env."""
    for env_var in ("PR_QUEUE_DATA_DIR", "ANTIGRAVITY_EXECUTABLE_DATA_DIR"):
        override = os.environ.get(env_var, "").strip()
        if override:
            return Path(override).expanduser().resolve()
    return (resolve_app_data_dir(home_dir) / "pr_queue").resolve()


def resolve_sidecar_token() -> str:
    """Returns the shared token injected by the Antigravity gateway, or empty string."""
    return os.environ.get("ANTIGRAVITY_SIDECAR_UI_TOKEN", "").strip()


def default_preload_js_candidates(
    app_data_dir: Path, home_dir: Path
) -> tuple[Path, ...]:
    """Builds ordered candidate paths for Antigravity's sidecar_sdk `preload.js`."""
    # Prefer Antigravity's runtime install, then user config node_modules.
    return (
        app_data_dir / "lib" / "node_modules" / "sidecar_sdk" / "preload.js",
        home_dir / ".gemini" / "config" / "node_modules" / "sidecar_sdk" / "preload.js",
    )


def _uri_to_path(uri: str) -> Path | None:
    if not uri:
        return None
    if uri.startswith("file://"):
        parsed = urlparse(uri)
        return Path(unquote(parsed.path)).resolve()
    candidate = Path(uri).expanduser()
    if candidate.is_absolute():
        return candidate.resolve()
    return None


def _parse_bool_field(raw: dict[str, Any], key: str, default: bool) -> bool:
    """Extracts an optional boolean field, rejecting non-boolean JSON values."""
    if key not in raw or raw[key] is None:
        return default
    val = raw[key]
    if not isinstance(val, bool):
        raise PrQueueError(f"`{key}` must be a boolean.")
    return val


async def _run_git(
    repo_dir: Path,
    args: Sequence[str],
    *,
    timeout_seconds: int = _GIT_LOCAL_TIMEOUT_SECONDS,
) -> str:
    """Runs a git subprocess under asyncio and returns stdout."""
    label = f"git {' '.join(args[:3])}"
    returncode, stdout, stderr = await run_subprocess_async(
        ["git", "-C", str(repo_dir), *args],
        label=label,
        timeout_seconds=timeout_seconds,
        error_cls=PrQueueError,
        missing_binary_message="`git` executable was not found on PATH.",
    )
    if returncode != 0:
        raise PrQueueError(format_subprocess_error(label, returncode, stdout, stderr))
    return stdout


async def _run_git_fetch(repo_dir: Path, args: Sequence[str]) -> str:
    """Runs `git fetch` with bounded retries on transient transport errors."""
    label = f"git fetch {' '.join(args[:2])}"
    return await retry_async(
        lambda: _run_git(repo_dir, args, timeout_seconds=_GIT_FETCH_TIMEOUT_SECONDS),
        is_retryable=lambda exc: (
            isinstance(exc, PrQueueError) and is_transient_error(str(exc))
        ),
        label=label,
    )


async def _main_worktree_root(path: Path) -> Path | None:
    """Returns the primary repository checkout root for any path inside a repo."""
    if not path.exists():
        return None
    try:
        output = await _run_git(path, ["worktree", "list", "--porcelain"])
    except PrQueueError:
        return None
    for line in output.splitlines():
        if line.startswith("worktree "):
            return Path(line[len("worktree ") :].strip()).resolve()
    return None


async def _remote_urls(repo_dir: Path) -> dict[str, str]:
    """Maps git remote names to their configured URLs for repo_dir."""
    try:
        # Read raw remote.<name>.url entries so `url.*.insteadOf` transport
        # rewrites still match the canonical GitHub owner/repo slug.
        output = await _run_git(
            repo_dir, ["config", "--get-regexp", r"^remote\..+\.url$"]
        )
    except PrQueueError:
        return {}
    remotes: dict[str, str] = {}
    for line in output.splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2:
            key, url = parts
            if key.startswith("remote.") and key.endswith(".url"):
                name = key[len("remote.") : -len(".url")]
                if name and name not in remotes:
                    remotes[name] = url.strip()
    return remotes


def _remote_matches_repo(remote_url: str, repo: str) -> bool:
    """Returns whether remote_url points to `github.com/<owner>/<repo>`."""
    raw = remote_url.strip()
    target = repo.strip().lower()
    if not raw or not target:
        return False
    if "://" in raw:
        parsed = urlsplit(raw)
        if (parsed.hostname or "").lower() != "github.com":
            return False
        path = parsed.path.lstrip("/").rstrip("/")
    else:
        ssh_match = re.match(r"^(?:[^@/:]+@)?github\.com:(.+)$", raw, re.IGNORECASE)
        if not ssh_match:
            return False
        path = ssh_match.group(1).lstrip("/").rstrip("/")
    if path.lower().endswith(".git"):
        path = path[:-4]
    return path.lower() == target


async def _list_and_read_projects(
    ls_info: dict[str, Any],
    read_rpc: Callable[
        [dict[str, Any], str, dict[str, Any]], Awaitable[dict[str, Any]]
    ],
) -> list[dict[str, Any]]:
    """Lists all Antigravity project IDs and reads their full non-archived records."""
    list_resp = await read_rpc(ls_info, "ListProjects", {})
    project_ids = list_resp.get("projectIds")
    if not isinstance(project_ids, list) or not project_ids:
        return []
    read_resp = await read_rpc(ls_info, "ReadProjects", {"ids": project_ids})
    projects = read_resp.get("projects", [])
    if not isinstance(projects, list):
        return []
    return [
        project
        for project in projects
        if isinstance(project, dict) and not project.get("archived")
    ]


async def _collect_project_paths_from_ls(
    daemon_dir: Path,
) -> list[Path]:
    """Queries Antigravity LS projects for registered workspace folder paths."""
    try:
        ls_info = resolve_ls_credentials(daemon_dir)
        projects = await _list_and_read_projects(
            ls_info, functools.partial(call_ls_rpc, retryable=True)
        )
    except PrQueueError as exc:
        _LOGGER.debug("Skipping LS project discovery: %s", exc)
        return []

    paths: list[Path] = []
    for project in projects:
        for uri in _extract_project_resource_uris(project):
            local = _uri_to_path(uri)
            if local and local not in paths:
                paths.append(local)
    return paths


def _collect_project_paths_from_disk(projects_file: Path) -> list[Path]:
    """Reads candidate folder paths from `projects.json` when LS is offline."""
    if not projects_file.is_file():
        return []
    try:
        data = json.loads(projects_file.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        _LOGGER.warning("Unable to parse %s: %s", projects_file, exc)
        return []

    paths: list[Path] = []

    def _visit(node: Any) -> None:
        if isinstance(node, dict):
            for key, value in node.items():
                if isinstance(key, str) and (
                    key.startswith("/") or key.startswith("file://")
                ):
                    parsed = _uri_to_path(key)
                    if parsed and parsed not in paths:
                        paths.append(parsed)
                if key in ("folderUri", "path", "root", "folder") and isinstance(
                    value, str
                ):
                    parsed = _uri_to_path(value)
                    if parsed and parsed not in paths:
                        paths.append(parsed)
                else:
                    _visit(value)
        elif isinstance(node, list):
            for item in node:
                _visit(item)

    _visit(data)
    return paths


async def resolve_base_repo(
    configured_base_repo: str,
    repo: str,
    *,
    projects_file: Path,
    daemon_dir: Path,
) -> Path:
    """Resolves the local main git repository matching the target GitHub repo."""
    if configured_base_repo.strip():
        explicit = Path(configured_base_repo.strip()).expanduser().resolve()
        if not explicit.exists():
            raise PrQueueError(f"Configured base repo path does not exist: {explicit}")
        root = await _main_worktree_root(explicit)
        if root is None:
            raise PrQueueError(
                f"Configured base repo is not a git repository: {explicit}"
            )
        return root

    candidates: list[Path] = []
    # Check ancestors of the plugin directory first (fast in worktree checkouts).
    for ancestor in [_SIDECAR_DIR, *_SIDECAR_DIR.parents]:
        if (ancestor / ".git").exists():
            ancestor_root = await _main_worktree_root(ancestor)
            if ancestor_root is not None:
                ancestor_remotes = await _remote_urls(ancestor_root)
                if any(
                    _remote_matches_repo(url, repo) for url in ancestor_remotes.values()
                ):
                    return ancestor_root
            candidates.append(ancestor)
            break

    # Fall back from live LS ReadProjects to ~/.gemini/projects.json if needed.
    ls_paths, disk_paths = await asyncio.gather(
        _collect_project_paths_from_ls(daemon_dir),
        asyncio.to_thread(_collect_project_paths_from_disk, projects_file),
    )
    candidates.extend(ls_paths)
    candidates.extend(disk_paths)

    roots = await asyncio.gather(
        *(_main_worktree_root(candidate) for candidate in candidates)
    )
    seen_roots: list[Path] = []
    for root in roots:
        if root is not None and root not in seen_roots:
            seen_roots.append(root)

    remote_maps = await asyncio.gather(*(_remote_urls(root) for root in seen_roots))
    for root, remotes in zip(seen_roots, remote_maps):
        if any(_remote_matches_repo(url, repo) for url in remotes.values()):
            return root

    raise PrQueueError(
        f"Could not auto-detect a local git checkout for `{repo}`. "
        "Open the repository in Antigravity or set `Base Repo` in Filters."
    )


async def detect_remote_for_repo(main_repo: Path, repo: str) -> str:
    """Selects the git remote on main_repo matching `github.com/<repo>`."""
    remotes = await _remote_urls(main_repo)
    matching = [
        name for name, url in remotes.items() if _remote_matches_repo(url, repo)
    ]
    for preferred in ("upstream", "origin"):
        if preferred in matching:
            return preferred
    if matching:
        return matching[0]
    raise PrQueueError(f"No git remote matching `{repo}` configured in {main_repo}.")


async def prepare_pr_worktree(
    main_repo: Path,
    pr_number: int,
    repo: str,
    base_ref: str = "main",
    *,
    worktrees_dir: Path,
    buildbuddy_rc: Path,
) -> tuple[Path, str, str]:
    """Creates or updates an isolated review worktree checked out at PR head.

    Args:
      main_repo: Primary local Git clone directory.
      pr_number: GitHub pull request number to check out.
      repo: Target repository slug (`owner/name`).
      base_ref: Base branch name (e.g., `main`).
      worktrees_dir: Base directory where per-repo review worktrees live.
      buildbuddy_rc: User BuildBuddy bazelrc symlinked into Bazel worktrees.

    Returns:
      Tuple `(worktree_path, head_sha, remote_name)`.
    """
    remote = await detect_remote_for_repo(main_repo, repo)
    clean_base = base_ref.strip() or "main"

    # Fetch base branch first so FETCH_HEAD remains available for PR ref next.
    # Force-update (+) the tracking ref in case the upstream branch rewrote history.
    await _run_git_fetch(
        main_repo,
        [
            "fetch",
            remote,
            f"+refs/heads/{clean_base}:refs/remotes/{remote}/{clean_base}",
        ],
    )
    await _run_git_fetch(
        main_repo,
        ["fetch", remote, f"pull/{pr_number}/head"],
    )
    head_sha = (await _run_git(main_repo, ["rev-parse", "FETCH_HEAD"])).strip()
    if not head_sha:
        raise PrQueueError(
            f"Fetched PR #{pr_number} head from `{remote}`, but FETCH_HEAD was empty."
        )

    await _run_git(main_repo, ["worktree", "prune"])
    worktree_path = (
        worktrees_dir / main_repo.name / f"review-pr-{pr_number}"
    ).resolve()
    branch_name = f"review-pr-{pr_number}"

    if worktree_path.exists():
        if not (worktree_path / ".git").exists():
            raise PrQueueError(
                f"Review path exists and is not a git worktree: {worktree_path}"
            )
        status = (await _run_git(worktree_path, ["status", "--porcelain"])).strip()
        if status:
            raise PrQueueError(
                f"Review worktree has uncommitted changes at {worktree_path}; "
                "clean or remove it before launching a fresh review."
            )
        # -B resets an existing review branch cleanly to the latest PR head SHA.
        await _run_git(worktree_path, ["checkout", "-B", branch_name, head_sha])
    else:
        worktree_path.parent.mkdir(parents=True, exist_ok=True)
        # -B resets the review branch name if it already exists from an older review.
        await _run_git(
            main_repo,
            ["worktree", "add", "-B", branch_name, str(worktree_path), head_sha],
        )

    user_bazelrc = worktree_path / "user.bazelrc"
    # Symlink user's BuildBuddy cache config so Bazel runs match local builds.
    if (
        buildbuddy_rc.is_file()
        and (worktree_path / "MODULE.bazel").is_file()
        and not user_bazelrc.exists()
    ):
        try:
            user_bazelrc.symlink_to(buildbuddy_rc)
        except OSError as exc:
            _LOGGER.warning(
                "Could not symlink BuildBuddy config %s: %s", buildbuddy_rc, exc
            )

    return (worktree_path, head_sha, remote)


def _parse_ls_address(raw_addr: str) -> tuple[str, str, int] | None:
    """Parses ANTIGRAVITY_LS_ADDRESS (`host:port` or `http(s)://host:port`)."""
    addr = raw_addr.strip()
    if not addr:
        return None
    if "://" not in addr:
        host, _, port_str = addr.rpartition(":")
        if not port_str.isdigit():
            return None
        port = int(port_str)
        if port <= 0 or port > 65535:
            return None
        return ("http", host or "127.0.0.1", port)
    parsed = urlsplit(addr)
    scheme = parsed.scheme.lower()
    if scheme not in ("http", "https") or not parsed.port:
        return None
    return (scheme, parsed.hostname or "127.0.0.1", parsed.port)


def resolve_ls_credentials(daemon_dir: Path | None = None) -> dict[str, Any]:
    """Discovers a running local Antigravity language server endpoint and CSRF token."""
    ls_addr = os.environ.get("ANTIGRAVITY_LS_ADDRESS", "").strip()
    ls_csrf = os.environ.get("ANTIGRAVITY_CSRF_TOKEN", "").strip()
    parsed_addr = _parse_ls_address(ls_addr)
    if parsed_addr and ls_csrf:
        scheme, host, port = parsed_addr
        return {
            "scheme": scheme,
            "host": host,
            "port": port,
            "csrf_token": ls_csrf,
        }

    # Standalone launches outside Antigravity's supervisor lack LS env vars, so
    # fall back to Antigravity's daemon registration files on disk.
    effective_daemon_dir = (
        resolve_app_data_dir() / "daemon" if daemon_dir is None else daemon_dir
    )
    candidates: list[tuple[float, Path]] = []
    if effective_daemon_dir.is_dir():
        for entry in effective_daemon_dir.glob("*.json"):
            with contextlib.suppress(OSError):
                candidates.append((entry.stat().st_mtime, entry))

    candidates.sort(key=lambda item: item[0], reverse=True)
    for _mtime, entry in candidates:
        try:
            payload = json.loads(entry.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if not isinstance(payload, dict):
            continue
        pid = payload.get("pid")
        csrf = str(payload.get("csrfToken") or "").strip()
        if not csrf:
            continue
        if isinstance(pid, int) and pid > 0:
            try:
                os.kill(pid, 0)
            except OSError:
                continue
        for scheme, key in (("http", "httpPort"), ("https", "httpsPort")):
            port = payload.get(key)
            if isinstance(port, int) and port > 0:
                return {
                    "scheme": scheme,
                    "host": "127.0.0.1",
                    "port": port,
                    "csrf_token": csrf,
                }

    raise PrQueueError(
        f"Could not locate a running Antigravity LS in {effective_daemon_dir}."
    )


def _call_ls_rpc_sync(
    ls_info: dict[str, Any],
    method: str,
    body: dict[str, Any],
) -> dict[str, Any]:
    """Performs a synchronous Connect JSON RPC against Antigravity LS."""
    scheme = str(ls_info.get("scheme") or "http").lower()
    host = str(ls_info.get("host") or "127.0.0.1")
    port = int(ls_info["port"])
    csrf = str(ls_info["csrf_token"])
    url = f"{scheme}://{host}:{port}/exa.language_server_pb.LanguageServerService/{method}"
    encoded = json.dumps(body).encode("utf-8")
    req = Request(
        url,
        data=encoded,
        method="POST",
        headers={
            "Content-Type": "application/json",
            "Connect-Protocol-Version": "1",
            "x-codeium-csrf-token": csrf,
        },
    )
    ssl_context: ssl.SSLContext | None = None
    if scheme == "https":
        if host not in _LOOPBACK_HOSTS:
            raise PrQueueError(
                f"Refusing unverified TLS connection to non-loopback LS host: {host}"
            )
        # Localhost daemon certificates are self-signed ephemeral certs.
        ssl_context = ssl._create_unverified_context()

    try:
        with urlopen(req, timeout=_LS_RPC_TIMEOUT_SECONDS, context=ssl_context) as resp:
            raw = resp.read().decode("utf-8")
    except HTTPError as exc:
        detail = ""
        with contextlib.suppress(OSError, UnicodeDecodeError):
            detail = exc.read().decode("utf-8", errors="replace")
        msg = (
            f"Antigravity RPC {method} failed (HTTP {exc.code}): {detail or exc.reason}"
        )
        if exc.code in _TRANSIENT_LS_HTTP_CODES:
            raise _TransientLsRpcError(msg) from exc
        raise PrQueueError(msg) from exc
    except (URLError, OSError, TimeoutError) as exc:
        raise _TransientLsRpcError(
            f"Antigravity RPC {method} request error: {exc}"
        ) from exc

    if not raw.strip():
        return {}
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise PrQueueError(
            f"Antigravity RPC {method} returned invalid JSON: {exc}"
        ) from exc
    return parsed if isinstance(parsed, dict) else {}


async def call_ls_rpc(
    ls_info: dict[str, Any],
    method: str,
    body: dict[str, Any],
    *,
    retryable: bool = False,
) -> dict[str, Any]:
    """Invokes a Connect JSON RPC on Antigravity's LanguageServerService."""
    if not retryable:
        return await asyncio.to_thread(_call_ls_rpc_sync, ls_info, method, body)
    return await retry_async(
        lambda: asyncio.to_thread(_call_ls_rpc_sync, ls_info, method, body),
        is_retryable=lambda exc: isinstance(exc, _TransientLsRpcError),
        label=f"LS RPC {method}",
    )


def _extract_resource_uris_from_list(resources: Any) -> list[str]:
    uris: list[str] = []
    if not isinstance(resources, list):
        return uris
    for res in resources:
        if not isinstance(res, dict):
            continue
        direct = res.get("folderUri")
        if isinstance(direct, str) and direct:
            uris.append(direct)
        git_folder = res.get("gitFolder")
        if isinstance(git_folder, dict):
            gf_uri = git_folder.get("folderUri")
            if isinstance(gf_uri, str) and gf_uri:
                uris.append(gf_uri)
    return uris


def _extract_project_resource_uris(project: dict[str, Any]) -> list[str]:
    proj_res = project.get("projectResources")
    if isinstance(proj_res, dict) and "resources" in proj_res:
        return _extract_resource_uris_from_list(proj_res.get("resources"))
    return _extract_resource_uris_from_list(project.get("resources"))


def _project_environments(project: dict[str, Any]) -> list[dict[str, Any]]:
    envs_container = project.get("environments")
    if isinstance(envs_container, dict):
        raw_envs = envs_container.get("environments", [])
    elif isinstance(envs_container, list):
        raw_envs = envs_container
    else:
        return []
    if not isinstance(raw_envs, list):
        return []
    return [env for env in raw_envs if isinstance(env, dict)]


def _env_paths(env: dict[str, Any]) -> list[Path]:
    raw_resources = env.get("resources")
    env_resources = (
        raw_resources.get("resources", []) if isinstance(raw_resources, dict) else []
    )
    paths: list[Path] = []
    for uri in _extract_resource_uris_from_list(env_resources):
        parsed = _uri_to_path(uri)
        if parsed is not None:
            paths.append(parsed)
    return paths


def _match_project_for_repo(
    projects: Sequence[dict[str, Any]],
    resolved_main: Path,
    resolved_worktree: Path,
) -> dict[str, Any] | None:
    for project in projects:
        proj_paths = [
            _uri_to_path(uri) for uri in _extract_project_resource_uris(project)
        ]
        if resolved_worktree in proj_paths or resolved_main in proj_paths:
            return project
    for project in projects:
        for env in _project_environments(project):
            paths = _env_paths(env)
            if resolved_worktree in paths or resolved_main in paths:
                return project
    name_matches = [
        project
        for project in projects
        if str(project.get("name") or "") == resolved_main.name
    ]
    if len(name_matches) == 1:
        return name_matches[0]
    return None


async def _lookup_project_by_conversation(
    ls_info: dict[str, Any],
    read_rpc: Callable[
        [dict[str, Any], str, dict[str, Any]], Awaitable[dict[str, Any]]
    ],
    projects: Sequence[dict[str, Any]],
    source_conversation_id: str,
) -> dict[str, Any] | None:
    if not source_conversation_id:
        return None
    try:
        meta_resp = await read_rpc(
            ls_info,
            "GetConversationMetadata",
            {"conversationId": source_conversation_id},
        )
    except PrQueueError as exc:
        _LOGGER.warning(
            "GetConversationMetadata failed for %s: %s",
            source_conversation_id,
            exc,
        )
        return None
    metadata = meta_resp.get("metadata") if isinstance(meta_resp, dict) else None
    if not isinstance(metadata, dict):
        return None
    project_id = str(metadata.get("projectId") or "").strip()
    if not project_id or project_id == "outside-of-project":
        return None
    for project in projects:
        if str(project.get("id") or "").strip() == project_id:
            return project
    return None


async def ensure_project_environment(
    ls_info: dict[str, Any],
    main_repo: Path,
    worktree_path: Path,
    *,
    source_conversation_id: str = "",
    ls_caller: Callable[
        [dict[str, Any], str, dict[str, Any]], Awaitable[dict[str, Any]]
    ]
    | None = None,
) -> tuple[str, str]:
    """Finds or creates an Antigravity project environment for worktree_path."""
    read_rpc = (
        functools.partial(call_ls_rpc, retryable=True)
        if ls_caller is None
        else ls_caller
    )
    write_rpc = call_ls_rpc if ls_caller is None else ls_caller
    projects = await _list_and_read_projects(ls_info, read_rpc)

    resolved_main = main_repo.resolve()
    resolved_worktree = worktree_path.resolve()
    matched_project = _match_project_for_repo(
        projects, resolved_main, resolved_worktree
    )
    if matched_project is None and source_conversation_id:
        matched_project = await _lookup_project_by_conversation(
            ls_info, read_rpc, projects, source_conversation_id
        )
    if matched_project is None:
        raise PrQueueError(
            f"Could not find an Antigravity workspace project for `{resolved_main}`. "
            "Open the repository workspace in Antigravity before launching a review."
        )

    project_id = str(matched_project.get("id") or "").strip()
    if not project_id:
        raise PrQueueError(
            f"Matched Antigravity workspace project for `{resolved_main}` has no ID."
        )

    env_list = _project_environments(matched_project)
    for env in env_list:
        if resolved_worktree in _env_paths(env):
            env_id = str(env.get("id") or "").strip()
            if env_id:
                return (project_id, env_id)
    for env in env_list:
        if str(env.get("name") or "") == worktree_path.name:
            env_id = str(env.get("id") or "").strip()
            if env_id:
                return (project_id, env_id)

    new_env_id = str(uuid.uuid4())
    add_req = {
        "projectId": project_id,
        "environment": {
            "id": new_env_id,
            "name": worktree_path.name,
            "resources": {"resources": [{"folderUri": f"file://{resolved_worktree}"}]},
        },
    }
    add_resp = await write_rpc(ls_info, "AddEnvironmentToProject", add_req)

    returned_env = add_resp.get("environment")
    if isinstance(returned_env, dict) and returned_env.get("id"):
        return (project_id, str(returned_env["id"]))
    return (project_id, new_env_id)


def _extract_model_from_config_data(data: dict[str, Any]) -> str:
    override = data.get("defaultOverrideModelConfig")
    if isinstance(override, dict):
        model_or_alias = override.get("modelOrAlias")
        if isinstance(model_or_alias, dict):
            model_val = str(model_or_alias.get("model") or "").strip()
            if model_val:
                return model_val

    client_configs = data.get("clientModelConfigs")
    if isinstance(client_configs, list):
        for cfg in client_configs:
            if not isinstance(cfg, dict) or cfg.get("disabled"):
                continue
            model_or_alias = cfg.get("modelOrAlias")
            if isinstance(model_or_alias, dict):
                model_val = str(model_or_alias.get("model") or "").strip()
                if model_val:
                    return model_val
    return ""


async def _resolve_model_from_available_models(
    read_rpc: Callable[
        [dict[str, Any], str, dict[str, Any]], Awaitable[dict[str, Any]]
    ],
    ls_info: dict[str, Any],
) -> str:
    try:
        resp = await read_rpc(ls_info, "GetAvailableModels", {})
    except PrQueueError as exc:
        _LOGGER.warning("GetAvailableModels failed: %s", exc)
        return ""
    inner = resp.get("response") if isinstance(resp, dict) else None
    if not isinstance(inner, dict):
        return ""
    models = inner.get("models")
    if not isinstance(models, dict):
        return ""

    candidate_ids: list[str] = []
    default_agent_id = str(inner.get("defaultAgentModelId") or "").strip()
    if default_agent_id:
        candidate_ids.append(default_agent_id)
    tiered = inner.get("tieredModelIds")
    if isinstance(tiered, dict):
        for tier in ("pro", "flash", "flashLite"):
            entries = tiered.get(tier)
            if isinstance(entries, list):
                candidate_ids.extend(str(x) for x in entries if x)

    for model_id in candidate_ids:
        details = models.get(model_id)
        if isinstance(details, dict) and not details.get("disabled"):
            model_val = str(details.get("model") or "").strip()
            if model_val:
                return model_val

    for details in models.values():
        if isinstance(details, dict) and not details.get("disabled"):
            model_val = str(details.get("model") or "").strip()
            if model_val:
                return model_val
    return ""


async def resolve_default_model(
    ls_info: dict[str, Any],
    *,
    ls_caller: Callable[
        [dict[str, Any], str, dict[str, Any]], Awaitable[dict[str, Any]]
    ]
    | None = None,
) -> str:
    """Resolves a preferred model enum/ID from Antigravity model config RPCs."""
    read_rpc = (
        functools.partial(call_ls_rpc, retryable=True)
        if ls_caller is None
        else ls_caller
    )
    try:
        cfg_resp = await read_rpc(ls_info, "GetCascadeModelConfigData", {})
    except PrQueueError as exc:
        _LOGGER.warning(
            "GetCascadeModelConfigData failed; trying GetAvailableModels: %s", exc
        )
    else:
        if isinstance(cfg_resp, dict):
            model_val = _extract_model_from_config_data(cfg_resp)
            if model_val:
                return model_val

    fallback = await _resolve_model_from_available_models(read_rpc, ls_info)
    if fallback:
        return fallback
    raise PrQueueError(
        "Could not determine a default Antigravity model from the language server."
    )


def _tokenize_title_words(text: str) -> list[str]:
    words: list[str] = []
    for raw in text.split():
        cleaned = raw.strip(_TITLE_PUNCT_STRIP_CHARS).lower()
        if cleaned and _HAS_ALNUM_RE.search(cleaned):
            words.append(cleaned)
    return words


def format_conversation_title(pr_number: int, title: str) -> str:
    """Builds a concise `pr-<N>: <up to 5 caveman words>` conversation title."""
    clean = " ".join(title.split())
    stripped = _BRACKET_PREFIX_RE.sub("", clean).strip() or clean

    ctype = ""
    scope = ""
    rest = stripped
    match = _CONVENTIONAL_COMMIT_RE.match(stripped)
    if match:
        ctype = match.group("ctype").lower()
        scope = (match.group("scope") or "").strip()
        rest = match.group("rest").strip()

    raw_words = _tokenize_title_words(rest)
    caveman_words = [w for w in raw_words if w not in _CAVEMAN_STOPWORDS]
    selected = list(caveman_words)

    kept_ctype = False
    if ctype and ctype not in _DROP_CONVENTIONAL_TYPES:
        ctype_stem = ctype.rstrip("s")
        if not selected or not selected[0].startswith(ctype_stem):
            selected.insert(0, ctype)
            kept_ctype = True

    if not selected:
        selected = list(raw_words)

    if scope and len(selected) < _MAX_CONVERSATION_TITLE_WORDS:
        scope_words = [
            w
            for w in _tokenize_title_words(scope.replace(",", " "))
            if w not in _CAVEMAN_STOPWORDS and w not in selected
        ]
        insert_idx = 1 if kept_ctype else 0
        room = _MAX_CONVERSATION_TITLE_WORDS - len(selected)
        for idx, sw in enumerate(scope_words[:room]):
            selected.insert(insert_idx + idx, sw)

    description = " ".join(selected[:_MAX_CONVERSATION_TITLE_WORDS]) or "review changes"
    prefix = f"pr-{pr_number}: "
    if len(prefix) + len(description) <= _MAX_CONVERSATION_TITLE_CHARS:
        return f"{prefix}{description}"
    budget = max(10, _MAX_CONVERSATION_TITLE_CHARS - len(prefix) - 3)
    return f"{prefix}{description[:budget]}..."


def build_review_prompt(
    *,
    repo: str,
    pr_number: int,
    title: str,
    url: str,
    remote: str,
    base_ref: str,
    head_ref: str,
    author: str,
) -> str:
    """Constructs the initial prompt sent to the spawned review conversation."""
    clean_base = base_ref.strip() or "main"
    return (
        f"Run `/review-code` (following `{_REVIEW_SKILL_REL_PATH}`) to review "
        f"GitHub PR #{pr_number} in `{repo}`.\n\n"
        f"- Title: {title}\n"
        f"- Author: @{author}\n"
        f"- Branch: `{head_ref}` -> `{clean_base}`\n"
        f"- URL: {url}\n\n"
        f"This worktree is already checked out at the PR head commit, with "
        f"`{remote}/{clean_base}` fetched so "
        f"`git diff $(git merge-base HEAD {remote}/{clean_base})` reflects the "
        f"PR changes. Produce the full `code-review-results.md` report and "
        f"ask before posting comments or approving the PR."
    )


async def start_pr_review_conversation(
    *,
    main_repo: Path,
    worktree_path: Path,
    prompt: str,
    title: str,
    source_conversation_id: str = "",
    daemon_dir: Path | None = None,
    ls_caller: Callable[
        [dict[str, Any], str, dict[str, Any]], Awaitable[dict[str, Any]]
    ]
    | None = None,
    ls_resolver: Callable[[], dict[str, Any]] | None = None,
) -> tuple[str, str, str]:
    """Starts a new Antigravity conversation bound to worktree_path and sends prompt.

    Args:
      main_repo: Primary local Git repository path.
      worktree_path: Dedicated worktree directory checked out for the PR.
      prompt: Initial user message sent to the new conversation.
      title: Conversation title annotation.
      source_conversation_id: Optional active conversation ID from the side panel.
      daemon_dir: Antigravity daemon directory for local credential discovery.
      ls_caller: Optional async RPC caller override for testing.
      ls_resolver: Optional credentials resolver override for testing.

    Returns:
      Tuple `(conversation_id, project_id, environment_id)`.
    """
    rpc = call_ls_rpc if ls_caller is None else ls_caller
    resolver = (
        functools.partial(resolve_ls_credentials, daemon_dir=daemon_dir)
        if ls_resolver is None
        else ls_resolver
    )
    ls_info = resolver()
    project_id, env_id = await ensure_project_environment(
        ls_info,
        main_repo,
        worktree_path,
        source_conversation_id=source_conversation_id,
        ls_caller=ls_caller,
    )
    requested_model = await resolve_default_model(ls_info, ls_caller=ls_caller)
    planner_config: dict[str, Any] = {
        "planModel": requested_model,
        "requestedModel": {"model": requested_model},
    }

    start_req: dict[str, Any] = {
        "source": "CORTEX_TRAJECTORY_SOURCE_CASCADE_CLIENT",
        "trajectoryType": "CORTEX_TRAJECTORY_TYPE_CASCADE",
        "projectEnvConfig": {
            "projectId": project_id,
            "environmentId": env_id,
        },
        "requestedModel": requested_model,
    }

    start_resp = await rpc(ls_info, "StartCascade", start_req)
    cascade_id = str(start_resp.get("cascadeId") or "").strip()
    if not cascade_id:
        raise PrQueueError("Antigravity `StartCascade` did not return a cascadeId.")

    if title:
        try:
            await rpc(
                ls_info,
                "UpdateConversationAnnotations",
                {
                    "cascadeIds": [cascade_id],
                    "annotations": {"title": title},
                    "mergeAnnotations": True,
                },
            )
        except PrQueueError as exc:
            _LOGGER.warning(
                "Failed setting title for conversation %s: %s", cascade_id, exc
            )

    await rpc(
        ls_info,
        "SendUserCascadeMessage",
        {
            "cascadeId": cascade_id,
            "items": [{"text": prompt}],
            "cascadeConfig": {"plannerConfig": planner_config},
            "blocking": False,
        },
    )
    return (cascade_id, project_id or "", env_id or "")


def _resolve_sqlite_columns(
    col_names: set[str],
) -> tuple[str, str, list[str], str, str] | None:
    """Resolves id, title, searchable, SELECT, and ORDER BY columns."""
    id_col = next(
        (c for c in ("cascade_id", "conversation_id", "id") if c in col_names),
        None,
    )
    title_col = next(
        (c for c in ("title", "name", "summary") if c in col_names),
        None,
    )
    if not id_col or not title_col:
        return None
    searchable_cols = list(
        dict.fromkeys(
            c
            for c in (title_col, "summary", "workspace_uri", "metadata")
            if c in col_names
        )
    )
    time_cols = [
        c for c in ("updated_at", "last_modified_time", "created_at") if c in col_names
    ]
    select_cols = ", ".join(dict.fromkeys([id_col, title_col, *time_cols]))
    order_col = time_cols[0] if time_cols else id_col
    return (id_col, title_col, searchable_cols, select_cols, order_col)


def _query_sqlite_pr_conversations(
    db_path: Path,
    repo: str,
    pr_number: int,
) -> list[dict[str, Any]]:
    """Searches Antigravity's conversation_summaries.db read-only for PR matches."""
    if not db_path.is_file():
        return []
    uri = f"file:{db_path}?mode=ro"
    matches: list[dict[str, Any]] = []
    conn: sqlite3.Connection | None = None
    try:
        conn = sqlite3.connect(uri, uri=True, timeout=_SQLITE_BUSY_TIMEOUT_MS / 1000.0)
        conn.row_factory = sqlite3.Row
        tables = {
            row[0]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type IN ('table', 'view')"
            )
        }
        if "conversation_summaries" not in tables:
            return []

        cols_info = conn.execute("PRAGMA table_info(conversation_summaries)").fetchall()
        col_names = {str(row["name"]) for row in cols_info}
        resolved = _resolve_sqlite_columns(col_names)
        if resolved is None:
            return []
        id_col, title_col, searchable_cols, select_cols, order_col = resolved

        pr_token = f"#{pr_number}"
        pr_prefix_token = f"pr-{pr_number}"
        pr_url_part = f"/{repo}/pull/{pr_number}"
        single_clause = " OR ".join(
            f"CAST({col} AS TEXT) LIKE ?" for col in searchable_cols
        )
        where_clauses = " OR ".join(f"({single_clause})" for _ in range(3))
        params = (
            [f"%{pr_token}%"] * len(searchable_cols)
            + [f"%{pr_prefix_token}%"] * len(searchable_cols)
            + [f"%{pr_url_part}%"] * len(searchable_cols)
        )

        query = (
            f"SELECT {select_cols} FROM conversation_summaries "
            f"WHERE {where_clauses} ORDER BY {order_col} DESC "
            f"LIMIT {_SQLITE_FILTER_CANDIDATE_LIMIT}"
        )
        rows = conn.execute(query, params).fetchall()
        pr_exact_re = re.compile(
            rf"(?<!\d)#{pr_number}(?!\d)"
            rf"|(?<![A-Za-z0-9])pr-{pr_number}(?!\d)"
            rf"|/pull/{pr_number}(?!\d)",
            re.IGNORECASE,
        )
        for row in rows:
            cid = str(row[id_col] or "").strip()
            ctitle = str(row[title_col] or "").strip()
            blob = " ".join(str(row[col] or "") for col in row.keys())
            if cid and pr_exact_re.search(blob):
                matches.append(
                    {
                        "conversation_id": cid,
                        "title": ctitle or f"Conversation {cid[:8]}",
                        "source": "history",
                    }
                )
    except sqlite3.Error as exc:
        _LOGGER.warning("SQLite conversation lookup failed in %s: %s", db_path, exc)
        return []
    finally:
        if conn is not None:
            conn.close()
    return matches


async def _fetch_single_pr(
    repo: str,
    pr_number: int,
    viewer: str,
    *,
    runner: Callable[[Sequence[str], int], Awaitable[str]] | None = None,
) -> dict[str, Any]:
    """Fetches metadata for a single PR number when not present in memory cache."""
    cmd_runner = run_gh_command_async if runner is None else runner
    raw = await cmd_runner(
        ["pr", "view", str(pr_number), "--repo", repo, "--json", PR_JSON_FIELDS],
        _SINGLE_PR_VIEW_TIMEOUT_SECONDS,
    )
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise PrQueueError(
            f"Invalid JSON from `gh pr view {pr_number}`: {exc}"
        ) from exc
    if not isinstance(parsed, dict) or "number" not in parsed:
        raise PrQueueError(f"PR #{pr_number} not found in `{repo}`.")
    summary = normalize_pr(
        parsed,
        repo=repo,
        viewer=viewer,
        max_age_days=DEFAULT_MAX_AGE_DAYS,
        now=datetime.datetime.now(datetime.UTC),
    )
    return summary.to_dict()


def _load_preload_js(candidates: Sequence[Path]) -> bytes | None:
    """Loads Antigravity's sidecar_sdk preload script from known local paths."""
    for candidate in candidates:
        if candidate.is_file():
            try:
                return candidate.read_bytes()
            except OSError as exc:
                _LOGGER.warning("Failed reading %s: %s", candidate, exc)
                continue
    return None


def _inject_preload(html_text: str) -> str:
    """Ensures index.html loads `/preload.js` once."""
    if "/preload.js" in html_text:
        return html_text
    tag = '<script src="/preload.js"></script>'
    if "</head>" in html_text:
        return html_text.replace("</head>", f"  {tag}\n</head>", 1)
    if "</body>" in html_text:
        return html_text.replace("</body>", f"  {tag}\n</body>", 1)
    return f"{tag}\n{html_text}"


# =============================================================================
# 4. Classes
# =============================================================================


class PrQueueError(Exception):
    """Raised on configuration, Git, GitHub, or Antigravity RPC errors."""


class _TransientLsRpcError(PrQueueError):
    """Raised on retryable Antigravity language server transport/5xx errors."""


class _HttpRequestError(PrQueueError):
    """Raised when an HTTP request is malformed or exceeds size limits."""

    def __init__(self, status: HTTPStatus, message: str) -> None:
        super().__init__(message)
        self.status = status


@dataclasses.dataclass
class PrQueueConfig:
    """User-configurable filter and repository settings for PR Queue."""

    # GitHub repository in owner/name format.
    repo: str = DEFAULT_REPO
    # GitHub login for viewer filters; empty resolves via `gh api user`.
    user: str = ""
    # Move PRs untouched for more than this many days to the Stale drawer; 0 = off.
    max_age_days: int = DEFAULT_MAX_AGE_DAYS
    # Include only PRs explicitly requesting a review from `user`.
    review_requested_only: bool = False
    # Hide PRs where `user`'s latest review is APPROVED.
    exclude_approved_by_me: bool = True
    # Hide PRs authored by `user`.
    exclude_authored_by_me: bool = True
    # Hide draft PRs.
    exclude_drafts: bool = True
    # Extra GitHub search filter passed to `gh pr list --search`.
    custom_query: str = ""
    # Optional override path to local git checkout; empty auto-detects.
    base_repo: str = ""

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> PrQueueConfig:
        """Creates a validated PrQueueConfig from a JSON dict."""
        defaults = cls()
        repo = str(raw.get("repo") or defaults.repo).strip() or defaults.repo
        if not _VALID_REPO_RE.match(repo):
            raise PrQueueError(
                f"Invalid repository format `{repo}`; expected `owner/name`."
            )
        raw_age = raw.get("max_age_days", defaults.max_age_days)
        if isinstance(raw_age, bool):
            raise PrQueueError("`max_age_days` must be a non-negative integer.")
        try:
            max_age_days = int(raw_age)
        except (TypeError, ValueError) as exc:
            raise PrQueueError(
                "`max_age_days` must be a non-negative integer."
            ) from exc
        if max_age_days < 0:
            raise PrQueueError("`max_age_days` must be a non-negative integer.")

        return cls(
            repo=repo,
            user=str(raw.get("user") or "").strip(),
            max_age_days=max_age_days,
            review_requested_only=_parse_bool_field(
                raw, "review_requested_only", defaults.review_requested_only
            ),
            exclude_approved_by_me=_parse_bool_field(
                raw, "exclude_approved_by_me", defaults.exclude_approved_by_me
            ),
            exclude_authored_by_me=_parse_bool_field(
                raw, "exclude_authored_by_me", defaults.exclude_authored_by_me
            ),
            exclude_drafts=_parse_bool_field(
                raw, "exclude_drafts", defaults.exclude_drafts
            ),
            custom_query=str(raw.get("custom_query") or "").strip(),
            base_repo=str(raw.get("base_repo") or "").strip(),
        )

    def to_dict(self) -> dict[str, Any]:
        """Serializes settings into a JSON-compatible dictionary."""
        return dataclasses.asdict(self)


@dataclasses.dataclass
class TriggeredPrRecord:
    """Metadata stored when the user launches a `/review-code` chat for a PR."""

    # GitHub pull request number.
    pr_number: int
    # GitHub repository in owner/name format.
    repo: str
    # Antigravity conversation (cascade) UUID.
    conversation_id: str
    # PR title at launch time.
    title: str
    # PR web URL.
    url: str
    # PR head commit SHA checked out when review launched.
    head_sha: str
    # PR source branch name.
    head_ref: str
    # PR author GitHub login.
    author: str
    # Filesystem path to the dedicated review worktree.
    worktree_path: str
    # Antigravity project ID associated with the conversation, if any.
    project_id: str
    # Antigravity environment ID created or reused for the worktree, if any.
    environment_id: str
    # Unix epoch timestamp when the review conversation was created.
    triggered_at: float

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> TriggeredPrRecord:
        """Reconstructs a TriggeredPrRecord from serialized JSON."""
        return cls(
            pr_number=int(raw["pr_number"]),
            repo=str(raw.get("repo") or DEFAULT_REPO),
            conversation_id=str(raw["conversation_id"]),
            title=str(raw.get("title") or ""),
            url=str(raw.get("url") or ""),
            head_sha=str(raw.get("head_sha") or ""),
            head_ref=str(raw.get("head_ref") or ""),
            author=str(raw.get("author") or ""),
            worktree_path=str(raw.get("worktree_path") or ""),
            project_id=str(raw.get("project_id") or ""),
            environment_id=str(raw.get("environment_id") or ""),
            triggered_at=float(raw.get("triggered_at") or 0.0),
        )

    def to_dict(self) -> dict[str, Any]:
        """Serializes record into a JSON-compatible dictionary."""
        return dataclasses.asdict(self)


class StateStore:
    """Thread-safe persistent storage for config, triggered chats, and cache."""

    def __init__(self, data_dir: Path) -> None:
        self._data_dir = data_dir
        self._data_dir.mkdir(parents=True, exist_ok=True)
        self._lock = threading.RLock()
        self._config_path = self._data_dir / "config.json"
        self._triggered_path = self._data_dir / "triggered_prs.json"
        self._cache_path = self._data_dir / "queue_cache.json"

    @property
    def data_dir(self) -> Path:
        """Returns the backing directory path for state files."""
        return self._data_dir

    @staticmethod
    def record_key(repo: str, pr_number: int) -> str:
        """Returns canonical key combining repo and PR number."""
        return f"{repo.lower()}#{pr_number}"

    def _read_json(self, path: Path, default: Any) -> Any:
        if not path.is_file():
            return default
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            _LOGGER.warning("Ignoring unreadable state file %s: %s", path, exc)
            return default

    def _write_json(self, path: Path, payload: Any) -> None:
        tmp_path = path.with_name(f".{path.name}.{uuid.uuid4().hex[:8]}.tmp")
        try:
            tmp_path.write_text(
                json.dumps(payload, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            os.replace(tmp_path, path)
        finally:
            with contextlib.suppress(OSError):
                if tmp_path.exists():
                    tmp_path.unlink()

    def get_config(self) -> PrQueueConfig:
        """Loads persisted config or returns defaults on missing/invalid data."""
        with self._lock:
            raw = self._read_json(self._config_path, {})
            if isinstance(raw, dict) and raw:
                try:
                    return PrQueueConfig.from_dict(raw)
                except PrQueueError as exc:
                    _LOGGER.warning(
                        "Invalid stored config %s (%s); using defaults.",
                        self._config_path,
                        exc,
                    )
                    return PrQueueConfig()
            return PrQueueConfig()

    def save_config(self, config: PrQueueConfig) -> PrQueueConfig:
        """Persists updated configuration atomically."""
        with self._lock:
            self._write_json(self._config_path, config.to_dict())
            return config

    def get_triggered_prs(self) -> dict[str, TriggeredPrRecord]:
        """Returns map of `repo#number` keys to TriggeredPrRecord entries."""
        with self._lock:
            raw = self._read_json(self._triggered_path, {})
            if not isinstance(raw, dict):
                return {}
            records: dict[str, TriggeredPrRecord] = {}
            for key, item in raw.items():
                if not isinstance(item, dict) or "conversation_id" not in item:
                    continue
                try:
                    records[str(key)] = TriggeredPrRecord.from_dict(item)
                except (KeyError, TypeError, ValueError) as exc:
                    _LOGGER.warning(
                        "Ignoring corrupt triggered PR record %r in %s: %s",
                        key,
                        self._triggered_path,
                        exc,
                    )
            return records

    def get_triggered_for_pr(
        self, repo: str, pr_number: int
    ) -> TriggeredPrRecord | None:
        """Looks up triggered review chat record for a PR in repo."""
        records = self.get_triggered_prs()
        return records.get(self.record_key(repo, pr_number))

    def record_triggered_pr(self, record: TriggeredPrRecord) -> None:
        """Saves or updates the triggered chat record for a PR."""
        with self._lock:
            raw = self._read_json(self._triggered_path, {})
            if not isinstance(raw, dict):
                raw = {}
            raw[self.record_key(record.repo, record.pr_number)] = record.to_dict()
            self._write_json(self._triggered_path, raw)

    def get_disk_cache(self) -> dict[str, Any] | None:
        """Loads the last persisted PR queue snapshot, if any."""
        with self._lock:
            raw = self._read_json(self._cache_path, None)
            return raw if isinstance(raw, dict) else None

    def save_disk_cache(self, payload: dict[str, Any]) -> None:
        """Persists a fetched PR queue snapshot to disk for offline fallback."""
        with self._lock:
            self._write_json(self._cache_path, payload)


class PrQueueService:
    """Coordinates manual queue fetching, conversation lookup, and chat launch."""

    def __init__(
        self,
        store: StateStore,
        *,
        worktrees_dir: Path,
        buildbuddy_rc: Path,
        conversations_db: Path,
        projects_file: Path,
        daemon_dir: Path,
        fetcher: Callable[..., Awaitable[PrQueueResult]] | None = None,
        gh_runner: Callable[[Sequence[str], int], Awaitable[str]] | None = None,
    ) -> None:
        self._store = store
        self._worktrees_dir = worktrees_dir
        self._buildbuddy_rc = buildbuddy_rc
        self._conversations_db = conversations_db
        self._projects_file = projects_file
        self._daemon_dir = daemon_dir
        self._fetcher = fetch_pr_queue_async if fetcher is None else fetcher
        self._gh_runner = run_gh_command_async if gh_runner is None else gh_runner
        self._refresh_lock = asyncio.Lock()
        self._launch_locks: dict[str, asyncio.Lock] = {}
        self._cached_queue: dict[str, Any] | None = None
        self._config_generation = 0

    @property
    def store(self) -> StateStore:
        """Returns the backing StateStore instance."""
        return self._store

    def _get_launch_lock(self, repo: str, pr_number: int) -> asyncio.Lock:
        key = StateStore.record_key(repo, pr_number)
        lock = self._launch_locks.get(key)
        if lock is None:
            lock = asyncio.Lock()
            self._launch_locks[key] = lock
        return lock

    async def get_config_payload(self) -> dict[str, Any]:
        """Returns current settings along with best-effort resolved local repo."""
        cfg = self._store.get_config()
        payload = cfg.to_dict()
        detected_repo = ""
        with contextlib.suppress(PrQueueError):
            detected_repo = str(
                await resolve_base_repo(
                    cfg.base_repo,
                    cfg.repo,
                    projects_file=self._projects_file,
                    daemon_dir=self._daemon_dir,
                )
            )
        payload["detected_base_repo"] = detected_repo
        return payload

    async def update_config(self, updates: dict[str, Any]) -> dict[str, Any]:
        """Updates stored config and invalidates in-memory cache so UI reloads."""
        current = self._store.get_config().to_dict()
        merged = {**current, **updates}
        new_cfg = PrQueueConfig.from_dict(merged)
        self._store.save_config(new_cfg)
        self._config_generation += 1
        self._cached_queue = None
        return await self.get_config_payload()

    def _attach_triggered_metadata(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Merges stored triggered review records onto PR items in payload."""
        repo = str(payload.get("repo") or DEFAULT_REPO)
        enriched = dict(payload)
        for list_key in ("prs", "stale_prs"):
            items = enriched.get(list_key)
            if not isinstance(items, list):
                continue
            updated_items: list[dict[str, Any]] = []
            for item in items:
                if not isinstance(item, dict):
                    continue
                copy_item = dict(item)
                pr_num = int(copy_item.get("number") or 0)
                rec = self._store.get_triggered_for_pr(repo, pr_num)
                if rec is not None:
                    copy_item["triggered_chat"] = {
                        "conversation_id": rec.conversation_id,
                        "title": rec.title,
                        "head_sha": rec.head_sha,
                        "worktree_path": rec.worktree_path,
                        "triggered_at": rec.triggered_at,
                        "stale_commit": bool(
                            copy_item.get("head_ref_oid")
                            and rec.head_sha
                            and copy_item.get("head_ref_oid") != rec.head_sha
                        ),
                    }
                else:
                    copy_item["triggered_chat"] = None
                updated_items.append(copy_item)
            enriched[list_key] = updated_items
        return enriched

    async def get_queue(self, *, refresh: bool = False) -> dict[str, Any]:
        """Returns cached PR queue, fetching from GitHub only when needed/requested."""
        if not refresh and self._cached_queue is not None:
            cached = self._attach_triggered_metadata(self._cached_queue)
            cached["from_cache"] = True
            return cached

        async with self._refresh_lock:
            if not refresh and self._cached_queue is not None:
                cached = self._attach_triggered_metadata(self._cached_queue)
                cached["from_cache"] = True
                return cached

            cfg = self._store.get_config()
            start_gen = self._config_generation
            try:
                result = await self._fetcher(
                    repo=cfg.repo,
                    user=cfg.user or None,
                    max_age_days=cfg.max_age_days,
                    review_requested_only=cfg.review_requested_only,
                    exclude_approved_by_me=cfg.exclude_approved_by_me,
                    exclude_authored_by_me=cfg.exclude_authored_by_me,
                    exclude_drafts=cfg.exclude_drafts,
                    custom_query=cfg.custom_query,
                    runner=self._gh_runner,
                )
            except FetchError as exc:
                _LOGGER.warning("PR queue fetch failed for %s: %s", cfg.repo, exc)
                fallback = self._cached_queue or self._store.get_disk_cache()
                if (
                    fallback is not None
                    and str(fallback.get("repo") or "").lower() == cfg.repo.lower()
                ):
                    enriched = self._attach_triggered_metadata(fallback)
                    enriched["error"] = str(exc)
                    enriched["from_cache"] = True
                    enriched["stale_cache"] = True
                    return enriched
                empty_payload = PrQueueResult(
                    repo=cfg.repo,
                    viewer=cfg.user,
                    prs=[],
                    stale_prs=[],
                    skipped_drafts=0,
                    skipped_authored=0,
                    skipped_approved=0,
                    max_age_days=cfg.max_age_days,
                ).to_dict()
                empty_payload["error"] = str(exc)
                empty_payload["from_cache"] = False
                empty_payload["stale_cache"] = False
                empty_payload["fetched_at"] = ""
                return empty_payload

            payload = result.to_dict()
            payload["fetched_at"] = (
                datetime.datetime.now(datetime.UTC).replace(microsecond=0).isoformat()
            )
            payload["error"] = ""
            payload["stale_cache"] = False
            if start_gen == self._config_generation:
                self._cached_queue = payload
                await asyncio.to_thread(self._store.save_disk_cache, payload)
            enriched = self._attach_triggered_metadata(payload)
            enriched["from_cache"] = False
            return enriched

    async def lookup_pr(self, pr_number: int) -> dict[str, Any]:
        """Finds tracked or historical review conversations for a PR number."""
        cfg = self._store.get_config()
        conversations: list[dict[str, Any]] = []
        seen_ids: set[str] = set()

        tracked = self._store.get_triggered_for_pr(cfg.repo, pr_number)
        if tracked is not None:
            seen_ids.add(tracked.conversation_id)
            conversations.append(
                {
                    "conversation_id": tracked.conversation_id,
                    "title": tracked.title,
                    "worktree_path": tracked.worktree_path,
                    "head_sha": tracked.head_sha,
                    "source": "tracked",
                }
            )

        history = await asyncio.to_thread(
            _query_sqlite_pr_conversations,
            self._conversations_db,
            cfg.repo,
            pr_number,
        )
        for match in history:
            cid = match["conversation_id"]
            if cid not in seen_ids:
                seen_ids.add(cid)
                conversations.append(match)

        return {
            "repo": cfg.repo,
            "pr_number": pr_number,
            "conversations": conversations,
        }

    async def get_conversation_metadata(self, conversation_id: str) -> dict[str, Any]:
        """Queries Antigravity LS `GetConversationMetadata` for conversation_id."""
        ls_info = resolve_ls_credentials(self._daemon_dir)
        meta = await call_ls_rpc(
            ls_info,
            "GetConversationMetadata",
            {"conversationId": conversation_id},
            retryable=True,
        )
        return {"response": {"conversationMetadata": meta}}

    def _find_cached_pr(self, repo: str, pr_number: int) -> dict[str, Any] | None:
        cache = self._cached_queue or self._store.get_disk_cache()
        if not cache:
            return None
        if str(cache.get("repo") or "").lower() != repo.lower():
            return None
        for key in ("prs", "stale_prs"):
            for item in cache.get(key, []):
                if isinstance(item, dict) and int(item.get("number") or 0) == pr_number:
                    return item
        return None

    async def _resolve_pr_metadata(
        self, cfg: PrQueueConfig, pr_number: int
    ) -> dict[str, Any]:
        cached = self._find_cached_pr(cfg.repo, pr_number)
        if cached is not None:
            return cached
        viewer = cfg.user
        if not viewer:
            with contextlib.suppress(FetchError):
                viewer = await get_current_gh_user_async(runner=self._gh_runner)
        try:
            return await _fetch_single_pr(
                cfg.repo, pr_number, viewer, runner=self._gh_runner
            )
        except FetchError as exc:
            raise PrQueueError(str(exc)) from exc

    async def launch_review(
        self,
        pr_number: int,
        *,
        force: bool = False,
        source_conversation_id: str = "",
        repo_resolver: Callable[[str, str], Awaitable[Path]] | None = None,
        worktree_preparer: Callable[
            [Path, int, str, str], Awaitable[tuple[Path, str, str]]
        ]
        | None = None,
        conversation_starter: Callable[..., Awaitable[tuple[str, str, str]]]
        | None = None,
    ) -> dict[str, Any]:
        """Creates/updates a worktree for pr_number and launches `/review-code`."""
        if pr_number <= 0:
            raise PrQueueError("`pr_number` must be a positive integer.")

        cfg = self._store.get_config()
        async with self._get_launch_lock(cfg.repo, pr_number):
            # Re-read config inside lock in case filters changed while waiting.
            cfg = self._store.get_config()
            existing = self._store.get_triggered_for_pr(cfg.repo, pr_number)
            if existing is not None and not force:
                return {
                    "ok": True,
                    "reused": True,
                    "pr_number": pr_number,
                    "conversation_id": existing.conversation_id,
                    "title": existing.title,
                    "worktree_path": existing.worktree_path,
                    "record": existing.to_dict(),
                }

            resolve_repo_fn = (
                functools.partial(
                    resolve_base_repo,
                    projects_file=self._projects_file,
                    daemon_dir=self._daemon_dir,
                )
                if repo_resolver is None
                else repo_resolver
            )
            prepare_wt_fn = (
                functools.partial(
                    prepare_pr_worktree,
                    worktrees_dir=self._worktrees_dir,
                    buildbuddy_rc=self._buildbuddy_rc,
                )
                if worktree_preparer is None
                else worktree_preparer
            )
            start_convo_fn = (
                functools.partial(
                    start_pr_review_conversation, daemon_dir=self._daemon_dir
                )
                if conversation_starter is None
                else conversation_starter
            )

            pr_meta = await self._resolve_pr_metadata(cfg, pr_number)
            main_repo = await resolve_repo_fn(cfg.base_repo, cfg.repo)
            base_ref = str(pr_meta.get("base_ref_name") or "main")
            head_ref = str(pr_meta.get("head_ref_name") or f"pr-{pr_number}")
            pr_title = str(pr_meta.get("title") or f"PR #{pr_number}")
            pr_url = str(
                pr_meta.get("url") or f"https://github.com/{cfg.repo}/pull/{pr_number}"
            )
            author = str(pr_meta.get("author") or "unknown")

            worktree_path, head_sha, remote = await prepare_wt_fn(
                main_repo, pr_number, cfg.repo, base_ref
            )
            conv_title = format_conversation_title(pr_number, pr_title)
            prompt = build_review_prompt(
                repo=cfg.repo,
                pr_number=pr_number,
                title=pr_title,
                url=pr_url,
                remote=remote,
                base_ref=base_ref,
                head_ref=head_ref,
                author=author,
            )

            start_kwargs: dict[str, Any] = {
                "main_repo": main_repo,
                "worktree_path": worktree_path,
                "prompt": prompt,
                "title": conv_title,
            }
            if source_conversation_id:
                start_kwargs["source_conversation_id"] = source_conversation_id
            conversation_id, project_id, environment_id = await start_convo_fn(
                **start_kwargs
            )

            record = TriggeredPrRecord(
                pr_number=pr_number,
                repo=cfg.repo,
                conversation_id=conversation_id,
                title=conv_title,
                url=pr_url,
                head_sha=head_sha,
                head_ref=head_ref,
                author=author,
                worktree_path=str(worktree_path),
                project_id=project_id,
                environment_id=environment_id,
                triggered_at=time.time(),
            )
            await asyncio.to_thread(self._store.record_triggered_pr, record)
            return {
                "ok": True,
                "reused": False,
                "pr_number": pr_number,
                "conversation_id": conversation_id,
                "title": conv_title,
                "worktree_path": str(worktree_path),
                "record": record.to_dict(),
            }


class PrQueueHttpServer:
    """Asyncio HTTP/1.1 server serving the PR Queue UI and JSON APIs."""

    def __init__(
        self,
        server_address: tuple[str, int],
        service: PrQueueService,
        *,
        html_path: Path,
        token: str,
        preload_candidates: Sequence[Path],
        allow_unauthenticated: bool = False,
    ) -> None:
        if not token and not allow_unauthenticated:
            raise PrQueueError(
                "ANTIGRAVITY_SIDECAR_UI_TOKEN is required unless "
                "--allow-unauthenticated is set."
            )
        self._host, self._port = server_address
        self.service = service
        self.html_path = html_path
        self.sidecar_token = token
        self._preload_candidates = tuple(preload_candidates)
        self._allow_unauthenticated = allow_unauthenticated
        self._server: asyncio.AbstractServer | None = None
        self.server_port: int = self._port
        self._client_tasks: set[asyncio.Task[None]] = set()

    async def start(self) -> int:
        """Starts listening on the configured TCP host and port."""
        if self._server is not None:
            return self.server_port
        self._server = await asyncio.start_server(
            self._accept_client, self._host, self._port
        )
        sockets = self._server.sockets
        if sockets:
            self.server_port = int(sockets[0].getsockname()[1])
        return self.server_port

    async def serve_forever(self) -> None:
        """Runs the server until cancelled or closed."""
        await self.start()
        assert self._server is not None  # type assert
        try:
            async with self._server:
                await self._server.serve_forever()
        finally:
            self._server = None
            await self.close()

    async def close(self) -> None:
        """Stops accepting connections and cancels any open client tasks."""
        if self._server is not None:
            self._server.close()
            await self._server.wait_closed()
            self._server = None
        pending = list(self._client_tasks)
        for task in pending:
            task.cancel()
        if pending:
            await asyncio.gather(*pending, return_exceptions=True)

    def _accept_client(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        task = asyncio.create_task(self._handle_client(reader, writer))
        self._client_tasks.add(task)
        task.add_done_callback(self._client_tasks.discard)

    async def _read_http_headers(
        self, reader: asyncio.StreamReader
    ) -> tuple[str, str, dict[str, str]] | None:
        raw_req = await asyncio.wait_for(
            reader.readline(), timeout=_HTTP_IO_TIMEOUT_SECONDS
        )
        if not raw_req:
            return None
        req_line = raw_req.decode("latin-1").rstrip("\r\n")
        parts = req_line.split(" ", 2)
        if len(parts) != 3 or not parts[1] or not parts[2].startswith("HTTP/"):
            raise _HttpRequestError(
                HTTPStatus.BAD_REQUEST, "Malformed HTTP request line."
            )
        method, target, _version = parts

        headers: dict[str, str] = {}
        total_bytes = len(raw_req)
        for _ in range(_MAX_HEADER_LINES):
            raw_hdr = await asyncio.wait_for(
                reader.readline(), timeout=_HTTP_IO_TIMEOUT_SECONDS
            )
            if not raw_hdr or raw_hdr in (b"\r\n", b"\n"):
                return (method.upper(), target, headers)
            total_bytes += len(raw_hdr)
            if total_bytes > _MAX_HEADER_BYTES:
                raise _HttpRequestError(
                    HTTPStatus.REQUEST_HEADER_FIELDS_TOO_LARGE,
                    "HTTP headers exceed size limit.",
                )
            line = raw_hdr.decode("latin-1").rstrip("\r\n")
            if ":" in line:
                name, value = line.split(":", 1)
                headers[name.strip().lower()] = value.strip()
        raise _HttpRequestError(
            HTTPStatus.REQUEST_HEADER_FIELDS_TOO_LARGE,
            "Too many HTTP headers.",
        )

    async def _read_request_body(
        self, reader: asyncio.StreamReader, headers: dict[str, str]
    ) -> bytes:
        transfer_encoding = headers.get("transfer-encoding", "").lower()
        if "chunked" in transfer_encoding:
            return await self._read_chunked_body(reader)

        length_header = headers.get("content-length")
        if length_header is None:
            return b""
        if not length_header.isdigit():
            raise _HttpRequestError(
                HTTPStatus.BAD_REQUEST, "Invalid Content-Length header."
            )
        length = int(length_header)
        if length > _MAX_POST_BODY_BYTES:
            raise _HttpRequestError(
                HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                "Request body size exceeds limit.",
            )
        if length == 0:
            return b""
        return await asyncio.wait_for(
            reader.readexactly(length), timeout=_HTTP_IO_TIMEOUT_SECONDS
        )

    async def _read_chunked_body(self, reader: asyncio.StreamReader) -> bytes:
        chunks: list[bytes] = []
        total = 0
        while True:
            size_line = await asyncio.wait_for(
                reader.readline(), timeout=_HTTP_IO_TIMEOUT_SECONDS
            )
            if not size_line:
                raise _HttpRequestError(
                    HTTPStatus.BAD_REQUEST, "Unexpected EOF reading chunked body."
                )
            size_token = size_line.decode("latin-1").split(";", 1)[0].strip()
            try:
                chunk_size = int(size_token, 16)
            except ValueError as exc:
                raise _HttpRequestError(
                    HTTPStatus.BAD_REQUEST, "Invalid chunk size in request body."
                ) from exc
            if chunk_size < 0:
                raise _HttpRequestError(
                    HTTPStatus.BAD_REQUEST, "Negative chunk size in request body."
                )
            if total + chunk_size > _MAX_POST_BODY_BYTES:
                raise _HttpRequestError(
                    HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                    "Request body size exceeds limit.",
                )
            if chunk_size == 0:
                while True:
                    trailer = await asyncio.wait_for(
                        reader.readline(), timeout=_HTTP_IO_TIMEOUT_SECONDS
                    )
                    if not trailer or trailer in (b"\r\n", b"\n"):
                        break
                return b"".join(chunks)
            data = await asyncio.wait_for(
                reader.readexactly(chunk_size),
                timeout=_HTTP_IO_TIMEOUT_SECONDS,
            )
            chunks.append(data)
            total += chunk_size
            crlf = await asyncio.wait_for(
                reader.readline(), timeout=_HTTP_IO_TIMEOUT_SECONDS
            )
            if crlf not in (b"\r\n", b"\n"):
                raise _HttpRequestError(
                    HTTPStatus.BAD_REQUEST, "Malformed chunk terminator."
                )

    @staticmethod
    def _json_response(status: HTTPStatus, data: Any) -> tuple[HTTPStatus, str, bytes]:
        body = (json.dumps(data) + "\n").encode("utf-8")
        return (status, "application/json; charset=utf-8", body)

    def _parse_json_body(self, raw_body: bytes) -> dict[str, Any]:
        raw = raw_body if raw_body else b"{}"
        try:
            parsed = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise PrQueueError(f"Malformed JSON request body: {exc}") from exc
        if not isinstance(parsed, dict):
            raise PrQueueError("Expected JSON object request body.")
        return parsed

    def _token_valid(self, headers: dict[str, str]) -> bool:
        if not self.sidecar_token:
            return self._allow_unauthenticated
        provided = headers.get("x-sidecar-token", "").strip()
        return bool(provided) and hmac.compare_digest(provided, self.sidecar_token)

    async def _handle_lookup_pr(
        self, query: dict[str, list[str]]
    ) -> tuple[HTTPStatus, str, bytes]:
        raw_pr = (query.get("pr") or [""])[0].strip().lstrip("#")
        try:
            pr_num = int(raw_pr)
            if pr_num <= 0:
                raise ValueError("non-positive")
        except ValueError:
            return self._json_response(
                HTTPStatus.BAD_REQUEST,
                {
                    "ok": False,
                    "error": "Query parameter `pr` must be a positive integer.",
                },
            )
        return self._json_response(HTTPStatus.OK, await self.service.lookup_pr(pr_num))

    async def _handle_review(
        self, body: dict[str, Any]
    ) -> tuple[HTTPStatus, str, bytes]:
        raw_pr_num = body.get("pr_number")
        if isinstance(raw_pr_num, (bool, float)) or raw_pr_num is None:
            raise PrQueueError("`pr_number` must be an integer.")
        try:
            pr_num = int(raw_pr_num)
        except (TypeError, ValueError) as exc:
            raise PrQueueError("`pr_number` must be an integer.") from exc
        force = _parse_bool_field(body, "force", False)
        raw_source_id = body.get("source_conversation_id")
        if raw_source_id is not None and not isinstance(raw_source_id, str):
            raise PrQueueError("`source_conversation_id` must be a string.")
        source_conversation_id = str(raw_source_id or "").strip()
        return self._json_response(
            HTTPStatus.OK,
            await self.service.launch_review(
                pr_num,
                force=force,
                source_conversation_id=source_conversation_id,
            ),
        )

    async def _dispatch_get(
        self, path: str, query: dict[str, list[str]]
    ) -> tuple[HTTPStatus, str, bytes]:
        if path in ("/", "/index.html"):
            if not self.html_path.is_file():
                return self._json_response(
                    HTTPStatus.NOT_FOUND, {"error": "index.html not found"}
                )
            html_text = await asyncio.to_thread(
                self.html_path.read_text, encoding="utf-8"
            )
            return (
                HTTPStatus.OK,
                "text/html; charset=utf-8",
                _inject_preload(html_text).encode("utf-8"),
            )

        if path == "/preload.js":
            js_bytes = await asyncio.to_thread(
                _load_preload_js, self._preload_candidates
            )
            if js_bytes is None:
                return self._json_response(
                    HTTPStatus.NOT_FOUND, {"error": "preload.js not found"}
                )
            return (
                HTTPStatus.OK,
                "application/javascript; charset=utf-8",
                js_bytes,
            )

        if path == "/healthz":
            return self._json_response(HTTPStatus.OK, {"ok": True})

        if path == "/api/config":
            return self._json_response(
                HTTPStatus.OK, await self.service.get_config_payload()
            )

        if path == "/api/queue":
            refresh_values = query.get("refresh")
            refresh_token = "0" if not refresh_values else refresh_values[0]
            if refresh_token not in ("0", "1"):
                return self._json_response(
                    HTTPStatus.BAD_REQUEST,
                    {
                        "ok": False,
                        "error": "Query parameter `refresh` must be `0` or `1`.",
                    },
                )
            queue_payload = await self.service.get_queue(refresh=(refresh_token == "1"))
            return self._json_response(HTTPStatus.OK, queue_payload)

        if path == "/api/lookup-pr":
            return await self._handle_lookup_pr(query)

        return self._json_response(
            HTTPStatus.NOT_FOUND, {"error": f"Unknown route: {path}"}
        )

    async def _dispatch_post(
        self, path: str, headers: dict[str, str], raw_body: bytes
    ) -> tuple[HTTPStatus, str, bytes]:
        if not self._token_valid(headers):
            return self._json_response(
                HTTPStatus.FORBIDDEN,
                {"ok": False, "error": "Missing or invalid X-Sidecar-Token."},
            )

        try:
            body = self._parse_json_body(raw_body)
            if path == "/_sidecar/get-conversation-metadata":
                conv_id = str(body.get("conversationId") or "").strip()
                if not conv_id:
                    raise PrQueueError("`conversationId` is required.")
                return self._json_response(
                    HTTPStatus.OK,
                    await self.service.get_conversation_metadata(conv_id),
                )

            if path == "/api/config":
                updated = await self.service.update_config(body)
                return self._json_response(
                    HTTPStatus.OK, {"ok": True, "config": updated}
                )

            if path == "/api/review":
                return await self._handle_review(body)
        except PrQueueError as exc:
            return self._json_response(
                HTTPStatus.BAD_REQUEST, {"ok": False, "error": str(exc)}
            )

        return self._json_response(
            HTTPStatus.NOT_FOUND, {"error": f"Unknown route: {path}"}
        )

    async def _dispatch_request(
        self,
        method: str,
        target: str,
        headers: dict[str, str],
        raw_body: bytes,
    ) -> tuple[HTTPStatus, str, bytes]:
        parsed = urlparse(target)
        path = parsed.path
        if method in ("GET", "HEAD"):
            return await self._dispatch_get(path, parse_qs(parsed.query))
        if method == "POST":
            return await self._dispatch_post(path, headers, raw_body)
        return self._json_response(
            HTTPStatus.METHOD_NOT_ALLOWED,
            {"error": f"Method not allowed: {method}"},
        )

    async def _send_response(
        self,
        writer: asyncio.StreamWriter,
        status: HTTPStatus,
        content_type: str,
        payload: bytes,
        *,
        head_only: bool = False,
    ) -> None:
        header_block = "\r\n".join(
            [
                f"HTTP/1.1 {int(status)} {status.phrase}",
                f"Content-Type: {content_type}",
                f"Content-Length: {len(payload)}",
                "Cache-Control: no-store",
                "Connection: close",
                "",
                "",
            ]
        ).encode("ascii")
        with contextlib.suppress(ConnectionResetError, BrokenPipeError, OSError):
            writer.write(header_block)
            if payload and not head_only:
                writer.write(payload)
            await writer.drain()

    async def _handle_client(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        try:
            try:
                req_meta = await self._read_http_headers(reader)
                if req_meta is None:
                    return
                method, target, headers = req_meta
                raw_body = (
                    await self._read_request_body(reader, headers)
                    if method == "POST"
                    else b""
                )
            except _HttpRequestError as exc:
                status, ctype, payload = self._json_response(
                    exc.status, {"ok": False, "error": str(exc)}
                )
                await self._send_response(writer, status, ctype, payload)
                return
            except asyncio.IncompleteReadError as exc:
                status, ctype, payload = self._json_response(
                    HTTPStatus.BAD_REQUEST,
                    {"ok": False, "error": f"Incomplete HTTP request body: {exc}"},
                )
                await self._send_response(writer, status, ctype, payload)
                return
            except (TimeoutError, ConnectionResetError, OSError):
                return

            try:
                status, ctype, payload = await self._dispatch_request(
                    method, target, headers, raw_body
                )
            # Boundary: return 500 JSON instead of dropping the socket.
            except Exception as exc:
                _LOGGER.exception("Unhandled error servicing %s %s", method, target)
                status, ctype, payload = self._json_response(
                    HTTPStatus.INTERNAL_SERVER_ERROR,
                    {"ok": False, "error": str(exc)},
                )
            await self._send_response(
                writer,
                status,
                ctype,
                payload,
                head_only=(method == "HEAD"),
            )
        finally:
            writer.close()
            with contextlib.suppress(OSError, asyncio.CancelledError):
                await writer.wait_closed()


# =============================================================================
# 5. Main
# =============================================================================


async def async_main(argv: Sequence[str] | None = None) -> int:
    """Runs the PR Queue sidecar HTTP server inside an asyncio event loop."""
    _configure_logging()
    args = parse_args(argv)
    home_dir = Path.home()
    app_data_dir = resolve_app_data_dir(home_dir)
    store = StateStore(resolve_data_dir(home_dir))
    service = PrQueueService(
        store,
        worktrees_dir=app_data_dir / "worktrees",
        buildbuddy_rc=home_dir / _DEFAULT_BUILDBUDDY_RC,
        conversations_db=app_data_dir / "conversation_summaries.db",
        projects_file=home_dir / _DEFAULT_PROJECTS_JSON,
        daemon_dir=app_data_dir / "daemon",
    )

    if args.dry_run:
        print(
            json.dumps(
                {
                    "data_dir": str(store.data_dir),
                    "config": await service.get_config_payload(),
                },
                indent=2,
            )
        )
        return 0

    try:
        server = PrQueueHttpServer(
            (args.host, args.port),
            service,
            html_path=_SIDECAR_DIR / "index.html",
            token=resolve_sidecar_token(),
            preload_candidates=default_preload_js_candidates(app_data_dir, home_dir),
            allow_unauthenticated=args.allow_unauthenticated,
        )
        bound_port = await server.start()
    except (PrQueueError, OSError) as exc:
        _LOGGER.error("Failed starting server on %s:%s: %s", args.host, args.port, exc)
        return 1

    print(
        f"PR Queue sidecar listening on http://{args.host}:{bound_port}",
        flush=True,
    )
    _LOGGER.info("PR Queue sidecar listening on http://%s:%d", args.host, bound_port)

    serve_task = asyncio.create_task(server.serve_forever())
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        with contextlib.suppress(NotImplementedError):
            loop.add_signal_handler(sig, serve_task.cancel)

    try:
        await serve_task
    except asyncio.CancelledError:
        pass
    finally:
        await server.close()
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    """Entry point starting the PR Queue asyncio sidecar HTTP server."""
    try:
        return asyncio.run(async_main(argv))
    except KeyboardInterrupt:
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
