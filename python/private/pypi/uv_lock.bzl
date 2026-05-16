"""Utility to convert a uv.lock file to JSON for use in rules_python."""

load("//python/private/pypi:pypi_repo_utils.bzl", "pypi_repo_utils")

def convert_uv_lock_to_json(mrctx, attr, logger):
    """Convert a uv.lock file to JSON using the toml2json tool.

    Uses the Python 3.14 interpreter to ensure tomllib is available.

    Args:
        mrctx: repository_ctx or module_ctx, used for file operations.
        attr: struct, the attributes of the rule, expected to have
            `_toml2json` and `uv_lock` or `srcs`.
        logger: struct, a simple logger for diagnostic messages.

    Returns:
        str, the JSON content of the uv.lock file as a string.
    """
    python_interpreter = mrctx.path(Label("@python_3_14//:python"))
    toml2json = mrctx.path(attr._toml2json)
    if hasattr(attr, "uv_lock") and attr.uv_lock:
        src_path = mrctx.path(attr.uv_lock)
    else:
        src_path = mrctx.path(attr.srcs[0])

    stdout = pypi_repo_utils.execute_checked_stdout(
        mrctx,
        logger = logger,
        op = "toml2json",
        python = python_interpreter,
        arguments = [
            str(toml2json),
            str(src_path),
        ],
        srcs = [toml2json, src_path],
    )
    return stdout
