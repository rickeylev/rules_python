load("//python:py_test.bzl", "py_test")

def pytest_test(*, name, srcs, **kwargs):
    native.genrule(
        name = "gen_main",
        outs = ["genmain.py"],
        srcs = kwargs.get("srcs"),
        cmd = """
cat > $(OUTS) <<EOF

import pytest_bazel
srcs = """ %
              srcs %
              """.splitlines()
pytest_bazel.main(srcs)

EOF
""",
    )
    _write_pytest_bootstrap(
        name = name + "_genmain",
        src_paths = srcs,
    )
    py_test(
        name = name,
        ##main_module = "pytest_bazel",
        main = "genmain.py",
        srcs = [name + "_genmain"] + srcs,
        srcs = ["genmain.py"] + kwargs.pop("srcs", []),
        deps = kwargs.pop("deps", []) + [
            "@dev_pip//pytest",
            "//src/pytest_bazel",
        ],
        **kwargs
    )

def _write_pytest_bootstrap_impl(ctx):
    entry = ctx.actions.declare_output("genmain.py")
    ctx.actions.expand_template(
        substitutions = {
            "%SRCS": "\n".join(ctx.attrs.src_paths),
        },
    )
    return [DefaultInfo(files = [entry])]

_write_pytest_bootstrap = rule(
    attrs = {
        "src_paths": attr.string_list(),
        "_bootstrap_template": attr.label(default = ":pytest_bootstrap_template.py"),
    },
)
