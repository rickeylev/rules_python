"""Aspect and rule definitions for Pyrefly static type checking."""

load("@rules_pyrefly//pyrefly:pyrefly.bzl", "pyrefly")

pyrefly_aspect = pyrefly()

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
