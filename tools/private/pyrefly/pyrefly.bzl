"""Aspect and rule definitions for Pyrefly static type checking."""

load("@rules_pyrefly//pyrefly:pyrefly.bzl", "pyrefly")

pyrefly_aspect = pyrefly()

def _pyrefly_check_test_impl(ctx):
    files = []
    for target in ctx.attr.targets:
        if OutputGroupInfo in target:
            files.append(target[OutputGroupInfo].pyrefly)

    test_bin = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        output = test_bin,
        content = "#!/bin/bash\nexit 0\n",
        is_executable = True,
    )
    return [
        DefaultInfo(
            executable = test_bin,
            runfiles = ctx.runfiles(transitive_files = depset(transitive = files)),
        ),
    ]

pyrefly_check_test = rule(
    implementation = _pyrefly_check_test_impl,
    test = True,
    doc = "Runs Pyrefly type checking on a list of targets as a Bazel test.",
    attrs = {
        "targets": attr.label_list(
            doc = "The target labels to type check.",
            mandatory = True,
            aspects = [pyrefly_aspect],
        ),
    },
)
