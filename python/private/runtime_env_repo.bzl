"""Internal setup to help the runtime_env toolchain."""

load("//python/private:repo_utils.bzl", "repo_utils")

def _runtime_env_repo_impl(rctx):
    pyenv = repo_utils.which_unchecked(rctx, "pyenv").binary
    if pyenv != None:
        ##main_project_python_version_file = rctx.path(rctx.attr._main_python_version_file)
        ##if main_project_python_version_file.exists:
        ##    cwd = str(main_project_python_version_file.dirname)
        ##    print("run pyenv in", cwd)
        ##else:
        ##    cwd = None
        pyenv_version_file = repo_utils.execute_checked(
            rctx,
            op = "GetPyenvVersionFile",
            arguments = [pyenv, "version-file"],
            ##    working_directory = cwd,
        ).stdout.strip()

        # When pyenv is used, the version file is what decided the
        # version used. Watch it so we compute the correct value if the
        # user changes it.
        rctx.watch(pyenv_version_file)
        print("watch:", pyenv_version_file)
        origin = repo_utils.execute_checked(
            rctx,
            op = "GetPyenvVersionFile",
            arguments = [pyenv, "version-origin"],
            ##working_directory = cwd,
        ).stdout.strip()
        print("origin:", origin)
        rctx.getenv("PYENV_VERSION")

    which = rctx.which("python3")
    print("py3 which:", which)
    rctx.execute
    version = repo_utils.execute_checked(
        rctx,
        op = "GetPythonVersion",
        arguments = [
            "python3",
            "-I",
            "-c",
            """import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")""",
        ],
        environment = {
            # Prevent the user's current shell from influencing the result.
            # This envvar won't be present when a test is run.
            # NOTE: This should be None, but Bazel 7 doesn't support None
            # values. Thankfully, pyenv treats empty string the same as missing.
            "PYENV_VERSION": "",
        },
    ).stdout.strip()
    print("Detected Python version:", version)
    rctx.file("info.bzl", "PYTHON_VERSION = '{}'\n".format(version))
    rctx.file("BUILD.bazel", "")

runtime_env_repo = repository_rule(
    implementation = _runtime_env_repo_impl,
    attrs = {
        "_main_python_version_file": attr.label(default = "@//:.python-version"),
    },
)
