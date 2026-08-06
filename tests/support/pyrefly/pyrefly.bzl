"""Aspect, rule, and macro definitions for Pyrefly static type checking."""

load("@bazel_skylib//rules:build_test.bzl", "build_test")
load("@rules_pyrefly//pyrefly:pyrefly.bzl", "pyrefly")
load("@rules_python//python/private:bzlmod_enabled.bzl", "BZLMOD_ENABLED")  # buildifier: disable=bzl-visibility

pyrefly_aspect = pyrefly(
    opt_in_tags = ["pyrefly"],
)

def _pyrefly_check_impl(ctx):
    files = []
    for target in ctx.attr.targets:
        if OutputGroupInfo in target:
            files.append(target[OutputGroupInfo].pyrefly)
    return [DefaultInfo(files = depset(transitive = files))]

pyrefly_check = rule(
    implementation = _pyrefly_check_impl,
    doc = "Runs Pyrefly type checking on a list of targets and collects diagnostic outputs.",
    attrs = {
        "targets": attr.label_list(
            doc = "The target labels to type check.",
            mandatory = True,
            aspects = [pyrefly_aspect],
        ),
    },
)

def pyrefly_check_test(name, targets, tags = None, **kwargs):
    """Macro that runs Pyrefly type checking on targets and tests it via build_test.

    Args:
        name: The name of the test target.
        targets: The list of targets to type check.
        tags: Optional tags to apply to the test target.
        **kwargs: Additional arguments forwarded to build_test.
    """

    # Pyrefly doesn't support WORKSPACE mode, so exit early. It is tested under Bzlmod.
    if not BZLMOD_ENABLED:
        return
    tags = tags or []
    check_name = "_" + name
    pyrefly_check(
        name = check_name,
        targets = targets,
        tags = ["manual"],
    )
    build_test(
        name = name,
        targets = [":" + check_name],
        tags = tags,
        **kwargs
    )
