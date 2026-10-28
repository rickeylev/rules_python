"""A WORKSPACE macro for installing PyPI dependencies from a uv.lock file."""

load("@bazel_skylib//lib:types.bzl", "types")
load("//python:versions.bzl", "MINOR_MAPPING")
load(":multi_pip_parse.bzl", "multi_pip_parse_hub_repo")
load(":uv_lock_repo.bzl", "uv_lock_repo")

def uv_lock(
        name,
        uv_lock,
        default_version = None,
        python_versions = None,
        python_interpreter_target = None,
        minor_mapping = None,
        **kwargs):
    """Accepts a resolved `uv.lock` file and installs the dependencies listed within.

    Supports both single-version and multi-version Python setups in `WORKSPACE`.

    :::{versionadded} VERSION_NEXT_FEATURE
    :::

    Args:
        name: {type}`str` The name of the generated PyPI hub repository.
        uv_lock: {type}`Label` A resolved `uv.lock` file.
        default_version: {type}`str | None` Optional default Python version when
            configuring multiple Python versions.
        python_versions: {type}`list[str] | None` Optional list of Python versions
            when `python_interpreter_target` is a dict. Defaults to the keys of
            `python_interpreter_target`.
        python_interpreter_target: {type}`Label | str | dict[str, Label] | None`
            Either a single target label for the host Python interpreter, or a
            dict mapping Python `X.Y` versions to host interpreter targets.
        minor_mapping: {type}`dict[str, str] | None` Optional mapping from `X.Y`
            to `X.Y.Z` Python versions for multi-version hub aliases. Defaults
            to `MINOR_MAPPING` from `//python:versions.bzl`.
        **kwargs: Additional repository rule arguments forwarded to each
            underlying PyPI repository (see {obj}`pip_parse` for supported
            attributes).
    """
    _uv_lock_repo = kwargs.pop("_uv_lock_repo", uv_lock_repo)
    _multi_pip_parse_hub_repo = kwargs.pop(
        "_multi_pip_parse_hub_repo",
        multi_pip_parse_hub_repo,
    )

    if not types.is_dict(python_interpreter_target):
        if python_interpreter_target != None:
            kwargs["python_interpreter_target"] = python_interpreter_target
        _uv_lock_repo(
            name = name,
            uv_lock = uv_lock,
            **kwargs
        )
        return

    python_versions = python_versions or sorted(python_interpreter_target.keys())
    minor_mapping = minor_mapping or MINOR_MAPPING
    pip_parses = {}
    for python_version in python_versions:
        if python_version not in python_interpreter_target:
            fail(
                "Missing python_interpreter_target for Python version %s in '%s'" % (
                    python_version,
                    name,
                ),
            )

        spoke_name = name + "_" + python_version.replace(".", "_")
        _uv_lock_repo(
            name = spoke_name,
            python_interpreter_target = python_interpreter_target[python_version],
            uv_lock = uv_lock,
            **kwargs
        )
        pip_parses[python_version] = spoke_name

    _multi_pip_parse_hub_repo(
        name = name,
        default_version = default_version,
        pip_parses = pip_parses,
        minor_mapping = minor_mapping,
    )
