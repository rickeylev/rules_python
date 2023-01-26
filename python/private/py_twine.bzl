"""Implementation of py_twine rule.

Simply wraps the tool with a bash script and a runfiles manifest.
See https://twine.readthedocs.io/
"""

load(":py_wheel.bzl", "PyWheelInfo")

_attrs = {
    "twine_bin": attr.label(
        doc = """\
A py_binary that runs the twine tool.

You might use the `entry_point` helper to supply the twine binary:
```starlark
load("@my_pip_parse_name//:requirements.bzl", "entry_point")
py_twine(
    ...
    twine_bin = entry_point("twine"),
)
```

Of course you can supply a py_binary by some other means which is CLI-compatible with twine.

Currently rules_python doesn't supply twine itself.
Follow https://github.com/bazelbuild/rules_python/issues/1016
""",
        executable = True,
        cfg = "exec",
    ),
    "wheel": attr.label(providers = [PyWheelInfo]),
    "_runfiles_lib": attr.label(default = "@bazel_tools//tools/bash/runfiles"),
}

# Bash helper function for looking up runfiles.
# Vendored from
# https://github.com/bazelbuild/bazel/blob/master/tools/bash/runfiles/runfiles.bash
BASH_RLOCATION_FUNCTION = r"""
# --- begin runfiles.bash initialization v2 ---
set -uo pipefail; f=bazel_tools/tools/bash/runfiles/runfiles.bash
source "${RUNFILES_DIR:-/dev/null}/$f" 2>/dev/null || \
source "$(grep -sm1 "^$f " "${RUNFILES_MANIFEST_FILE:-/dev/null}" | cut -f2- -d' ')" 2>/dev/null || \
source "$0.runfiles/$f" 2>/dev/null || \
source "$(grep -sm1 "^$f " "$0.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
source "$(grep -sm1 "^$f " "$0.exe.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
{ echo>&2 "ERROR: cannot find $f"; exit 1; }; f=; set -e
# --- end runfiles.bash initialization v2 ---
"""

# Copied from https://github.com/aspect-build/bazel-lib/blob/main/lib/private/paths.bzl
# to avoid a dependency from bazelbuild -> aspect-build
def _to_manifest_path(ctx, file):
    """The runfiles manifest entry path for a file.

    This is the full runfiles path of a file including its workspace name as
    the first segment. We refert to it as the manifest path as it is the path
    flavor that is used for in the runfiles MANIFEST file.
    We must avoid using non-normalized paths (workspace/../other_workspace/path)
    in order to locate entries by their key.
    Args:
        ctx: starlark rule execution context
        file: a File object
    Returns:
        The runfiles manifest entry path for a file
    """

    if file.short_path.startswith("../"):
        return file.short_path[3:]
    else:
        return ctx.workspace_name + "/" + file.short_path

_exec_tmpl = """\
#!/usr/bin/env bash
{rlocation}
tmp=$(mktemp -d)
# The namefile is just a file with one line, containing the real filename for the wheel.
wheel_filename=$tmp/$(cat "$(rlocation {wheel_namefile})")
cp $(rlocation {wheel}) $wheel_filename
$(rlocation {twine_bin}) upload $wheel_filename "$@"
"""

def _implementation(ctx):
    exec = ctx.actions.declare_file(ctx.label.name + ".sh")

    ctx.actions.write(exec, content = _exec_tmpl.format(
        rlocation = BASH_RLOCATION_FUNCTION,
        twine_bin = _to_manifest_path(ctx, ctx.executable.twine_bin),
        wheel = _to_manifest_path(ctx, ctx.files.wheel[0]),
        wheel_namefile = _to_manifest_path(ctx, ctx.attr.wheel[PyWheelInfo].name_file),
    ), is_executable = True)

    runfiles = ctx.runfiles(ctx.files.twine_bin + ctx.files.wheel + ctx.files._runfiles_lib + [
        ctx.attr.wheel[PyWheelInfo].name_file,
    ])
    runfiles = runfiles.merge(ctx.attr.twine_bin[DefaultInfo].default_runfiles)
    return [
        DefaultInfo(executable = exec, runfiles = runfiles),
    ]

py_twine_lib = struct(
    implementation = _implementation,
    attrs = _attrs,
)
