"""Defs for testing whl_library build file renaming."""

load("@bazel_skylib//rules:common_settings.bzl", "BuildSettingInfo")
load("@rules_python//python/pip_install:pip_repository.bzl", "whl_library") # Adjusted path

def _build_test_wheel_impl(rctx):
    # Find the python interpreter
    # Prefer python_interpreter_target if available (Bazel 7+)
    python_interpreter_target_attr = getattr(rctx.attr, "python_interpreter_target", None)
    if python_interpreter_target_attr:
        python_interpreter = rctx.executable._toolchain_python_interpreter if hasattr(rctx.executable, "_toolchain_python_interpreter") else rctx.executable.python_interpreter_target
    else:
        python_interpreter = rctx.executable.python_interpreter

    wheel_src_dir = rctx.path(rctx.attr.wheel_source_dir)
    out_dir = rctx.path(".") # Output to the repository root

    # Copy source files to a temporary directory to avoid issues with symlinks
    # and ensure hermeticity if setup.py/build process reads files multiple times.
    temp_src_dir = out_dir.get_child("temp_wheel_src")
    rctx.execute(["mkdir", "-p", temp_src_dir.path])
    # Copy contents of wheel_src_dir to temp_src_dir
    # Using find and cp to handle subdirectories correctly.
    copy_cmd = ("cd {} && find . -print0 | " +
                "xargs -0 -I {{}} cp --parents -rfL {{}} {}").format(
        wheel_src_dir.path,
        temp_src_dir.path,
    )
    copy_result = rctx.execute(["bash", "-c", copy_cmd], quiet = False)
    if copy_result.return_code != 0:
        fail("Failed to copy wheel sources: {}".format(copy_result.stderr))


    # Command to build the wheel using setuptools
    # Using python -m build if available, otherwise fallback to setup.py
    # Ensure the output directory for the wheel is clearly defined
    build_cmd = [
        python_interpreter.path,
        "-m",
        "pip", # Using pip to build
        "wheel",
        "--no-deps", # Don't fetch dependencies
        "--wheel-dir",
        out_dir.path, # Output wheel to the repository root
        temp_src_dir.path, # Source directory for building the wheel
    ]

    # Propagate environment variables that might be necessary for build tools
    # For example, those set by Bazel for toolchain discovery.
    env = {"LC_ALL": "en_US.UTF-8", "LANG": "en_US.UTF-8"} # Common locale settings
    # Add system environment variables that might be relevant for pip or build
    for var in ["PATH"]: # Add other vars if needed e.g. BAZEL_PYTHON
        sys_env_val = rctx.os.environ.get(var)
        if sys_env_val:
            env[var] = sys_env_val

    # For debugging build issues
    rctx.report_progress("Building wheel with command: {}".format(" ".join(build_cmd)))
    rctx.report_progress("Environment: {}".format(str(env)))
    rctx.report_progress("Source directory content (temp_src_dir):")
    rctx.execute(["ls", "-Ral", temp_src_dir.path])


    result = rctx.execute(
        build_cmd,
        # working_directory = temp_src_dir.path, # Build from the temp source dir
        environment = env,
        quiet = False, # Show output for debugging
    )

    if result.return_code != 0:
        fail("Failed to build wheel: \nSTDOUT:\n{}\nSTDERR:\n{}".format(result.stdout, result.stderr))

    # Find the generated wheel file (there should be only one)
    wheels_found = [f for f in out_dir.readdir() if f.name.endswith(".whl")]
    if not wheels_found:
        fail("No .whl file found in {} after build. stdout: {}, stderr: {}".format(out_dir, result.stdout, result.stderr))
    if len(wheels_found) > 1:
        fail("Multiple .whl files found: {}".format(wheels_found))

    built_wheel_name = wheels_found[0].name
    rctx.file("wheel_name.bzl", 'BUILT_WHEEL_NAME = "{}"'.format(built_wheel_name))

_build_test_wheel = repository_rule(
    implementation = _build_test_wheel_impl,
    attrs = {
        "wheel_source_dir": attr.label(mandatory = True, allow_single_file = False),
        "python_interpreter": attr.label(
            default = Label("@rules_python//python/runtime_env_toolchains:current_py_runtime_info"),
            executable = True,
            cfg = "exec",
        ),
        "python_interpreter_target": attr.label(
            default = Label("@rules_python//python/runtime_env_toolchains:current_py_runtime_info"),
            # providers = [PyRuntimeInfo], # This causes issues, use executable=True and common_settings
            executable = True, # This ensures it's treated as an executable
            cfg = "exec", # Execute in the execution configuration
        ),
        "_toolchain_python_interpreter": attr.label(
            default = "@bazel_tools//tools/python:autodetecting_toolchain",
            executable = True,
            cfg = "exec",
        ),
    },
)

def whl_build_file_renaming_test(name, whl_repo_name, site_packages_path="site-packages", data_files_path="data"):
    """
    Test macro to verify BUILD file renaming in a whl_library repository.

    Args:
        name: Name of the test.
        whl_repo_name: Name of the whl_library repository to inspect.
        site_packages_path: Path within the wheel repo to the site-packages dir.
        data_files_path: Path within the wheel repo to the data_files dir.
    """
    native.sh_test(
        name = name,
        srcs = ["test_renaming.sh"],
        args = [
            "$(location @{}/site-packages/pkg_with_build_files/_BUILD)".format(whl_repo_name),
            "$(location @{}/site-packages/pkg_with_build_files/__BUILD)".format(whl_repo_name), # Original BUILD, now __BUILD due to conflict
            "$(location @{}/site-packages/pkg_with_build_files/_BUILD.bazel)".format(whl_repo_name),
            "$(location @{}/{}/_BUILD)".format(whl_repo_name, data_files_path),
            # Negative checks (original files should not exist)
            "@{}/site-packages/pkg_with_build_files/BUILD".format(whl_repo_name),
            "@{}/site-packages/pkg_with_build_files/BUILD.bazel".format(whl_repo_name),
            "@{}/{}/BUILD".format(whl_repo_name, data_files_path),
            # Check that a file NOT named BUILD is still there
            "$(location @{}/site-packages/pkg_with_build_files/__init__.py)".format(whl_repo_name),
            "$(location @{}/site-packages/pkg_with_build_files/some_data.txt)".format(whl_repo_name),
            "$(location @{}/site-packages/pkg_with_build_files/another_data.txt)".format(whl_repo_name),
            "$(location @{}/site-packages/pkg_with_build_files/conflict_data.txt)".format(whl_repo_name),
            "$(location @{}/{}/data_specific_file.txt)".format(whl_repo_name, data_files_path),
        ],
        data = [
            "@{whl_repo_name}//{path}:_BUILD".format(whl_repo_name = whl_repo_name, path = site_packages_path + "/pkg_with_build_files"),
            "@{whl_repo_name}//{path}:__BUILD".format(whl_repo_name = whl_repo_name, path = site_packages_path + "/pkg_with_build_files"), # Original BUILD renamed due to conflict
            "@{whl_repo_name}//{path}:_BUILD.bazel".format(whl_repo_name = whl_repo_name, path = site_packages_path + "/pkg_with_build_files"),
            "@{whl_repo_name}//{data_path}:_BUILD".format(whl_repo_name = whl_repo_name, data_path = data_files_path),
            # Other files from the wheel to ensure they are still accessible
            "@{whl_repo_name}//{path}:__init__.py".format(whl_repo_name = whl_repo_name, path = site_packages_path + "/pkg_with_build_files"),
            "@{whl_repo_name}//{path}:some_data.txt".format(whl_repo_name = whl_repo_name, path = site_packages_path + "/pkg_with_build_files"),
            "@{whl_repo_name}//{path}:another_data.txt".format(whl_repo_name = whl_repo_name, path = site_packages_path + "/pkg_with_build_files"),
            "@{whl_repo_name}//{path}:conflict_data.txt".format(whl_repo_name = whl_repo_name, path = site_packages_path + "/pkg_with_build_files"),
            "@{whl_repo_name}//{data_path}:data_specific_file.txt".format(whl_repo_name = whl_repo_name, data_path = data_files_path),
        ],
        tags = ["requires-network"], # Building wheel might need network for pip/setuptools
    )

def testing_repositories():
    """Defines repositories needed for the test."""
    _build_test_wheel(
        name = "build_my_test_wheel",
        wheel_source_dir = "//tests/pypi/whl_library_repo_rule:testing_wheel_src",
    )

    # Load the built wheel name
    # This assumes wheel_name.bzl is available in the @build_my_test_wheel repo
    # This needs to be handled carefully for repository rule dependencies.
    # A common pattern is to write a known filename or use a filegroup.
    # For simplicity here, we'll assume the test setup knows the wheel name or can find it.
    # However, this dynamic loading is tricky.
    # A better way: whl_library should accept a label to a wheel file.

    # Since we know the name pattern, we can try to predict it, or have build_test_wheel output it
    # to a known file that whl_library can then use.
    # For now, let's assume the wheel name can be hardcoded or discovered by whl_library if it supports local paths.

    # The whl_library rule expects a requirement string or a direct whl_file label.
    # We will give it a label to the wheel built by _build_test_wheel.
    # This requires _build_test_wheel to expose the wheel file.
    # Let's modify _build_test_wheel to create a filegroup for the wheel.

    # Alternative: Make whl_library use the output of build_my_test_wheel
    # This would involve whl_library reading the wheel_name.bzl
    # For now, let's define a known output name in build_my_test_wheel or make it fixed.
    # Pip wheel usually creates versioned names.
    # The simplest for a test is to have whl_library point to a fixed name and
    # _build_test_wheel rule renames the output wheel to that fixed name.

    # Let's assume _build_test_wheel is modified to output "test_wheel.whl"
    # This is not done yet in the _build_test_wheel_impl above.
    # For now, we'll use a placeholder and refine. The ideal way is for _build_test_wheel
    # to provide the wheel as an output that whl_library can consume.

    # We need to load the actual wheel name from the file generated by _build_test_wheel
    # This is typically done in the WORKSPACE/MODULE.bazel file, not directly in a .bzl file
    # that defines tests.
    # The test rule itself will be defined in BUILD.bazel and will depend on the
    # @test_wheel_with_renamed_build_files repository.

    # This function is more for WORKSPACE setup.
    # The actual whl_library rule will be in WORKSPACE or a MODULE.bazel extension.
    pass
