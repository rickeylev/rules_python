# Copyright 2024 The Bazel Authors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Implementation of the `py_zipapp_binary` rule."""

load("@bazel_skylib//lib:paths.bzl", "paths")
load("//python/private:builders.bzl", "builders")
load("//python/private:common.bzl", "runfiles_root_path")
load("//python/private:py_executable_info.bzl", "PyExecutableInfo")
load("//python/private:py_info.bzl", "PyInfo")
load("//python/private:py_internal.bzl", "py_internal")
load("//python/private:py_interpreter_program.bzl", "PyInterpreterProgramInfo", "actions_run")
load("//python/private:py_runtime_info.bzl", "PyRuntimeInfo")
load("//python/private:toolchain_types.bzl", "EXEC_TOOLS_TOOLCHAIN_TYPE")

_ZIP_RUNFILES_DIRECTORY_NAME = "runfiles"
_EXTERNAL_PATH_PREFIX = "external"

def _create_stage2_bootstrap(ctx, py_executable):
    return py_executable.stage2_bootstrap

# This function is copied from py_executable.bzl to work around visibility rules.
# It has been renamed by removing the leading underscore to be locally callable.
def get_zip_runfiles_path_legacy(path, workspace_name, legacy_external_runfiles):
    def _get_zip_runfiles_path(p, ws_name = ""):
        if ws_name:
            return _ZIP_RUNFILES_DIRECTORY_NAME + "/" + ws_name + "/" + p
        else:
            return _ZIP_RUNFILES_DIRECTORY_NAME + "/" + p

    if legacy_external_runfiles and path.startswith(_EXTERNAL_PATH_PREFIX):
        return _get_zip_runfiles_path(path.removeprefix(_EXTERNAL_PATH_PREFIX))
    elif path.startswith("../"):
        return _get_zip_runfiles_path(path[3:])
    else:
        return _get_zip_runfiles_path(path, workspace_name)

def _create_zipapp_main_py(ctx, py_runtime, py_executable, stage2_bootstrap):
    python_exe = py_executable.venv_python_exe
    python_exe_path = runfiles_root_path(ctx, python_exe.short_path)

    if py_runtime.interpreter:
        python_binary_actual_path = runfiles_root_path(ctx, py_runtime.interpreter.short_path)
    else:
        python_binary_actual_path = py_runtime.interpreter_path

    zip_main_py = ctx.actions.declare_file(ctx.label.name + ".zip_main.py")
    ctx.actions.expand_template(
        template = py_runtime.zip_main_template,
        output = zip_main_py,
        substitutions = {
            # runfiles-root-relative path to venv's bin/python3. Empty if venv not being used.
            "%python_binary%": python_exe_path,
            # runfiles-root-relative path, absolute path, or single word. The actual Python
            # executable to use.
            "%python_binary_actual%": python_binary_actual_path,
            # runfiles-root relative path
            "%stage2_bootstrap%": runfiles_root_path(ctx, stage2_bootstrap.short_path),
            "%workspace_name%": ctx.workspace_name,
        },
    )
    return zip_main_py

def _build_manifest(ctx, manifest, runfiles, zip_main, output):
    workspace_name = ctx.workspace_name
    legacy_external_runfiles = py_internal.get_legacy_external_runfiles(ctx)

    manifest.add("0|__main__.py|{}".format(zip_main.path))

    empty_filename_lines = [
        get_zip_runfiles_path_legacy(path, workspace_name, legacy_external_runfiles) + "="
        for path in runfiles.empty_filenames.to_list()
    ]
    manifest.add_all(empty_filename_lines)

    # todo: need to have custom zipper program. The bazel zipper won't
    # handle symlinks correctly.
    runfile_lines = []
    for file in runfiles.files.to_list():
        line = get_zip_runfiles_path_legacy(file.short_path, workspace_name, legacy_external_runfiles) + "|" + file.path
        line = (str(int(file.is_symlink))) + "|" + line
        runfile_lines.append(line)
    manifest.add_all(runfile_lines)

    inputs = [zip_main]
    if py_internal.is_bzlmod_enabled(ctx):
        zip_repo_mapping_manifest = ctx.actions.declare_file(
            output.basename + ".repo_mapping",
            sibling = output,
        )
        py_internal.create_repo_mapping_manifest(
            ctx = ctx,
            runfiles = runfiles,
            output = zip_repo_mapping_manifest,
        )
        manifest.add("0|{}/_repo_mapping|{}".format(
            _ZIP_RUNFILES_DIRECTORY_NAME,
            zip_repo_mapping_manifest.path,
        ))
        inputs.append(zip_repo_mapping_manifest)
    return inputs

def _create_zip(ctx, py_runtime, py_executable, stage2_bootstrap):
    output = ctx.actions.declare_file(ctx.label.name + ".zip")
    manifest = ctx.actions.args()
    manifest.use_param_file("%s", use_always = True)
    manifest.set_param_file_format("multiline")

    print(py_executable)
    runfiles = builders.RunfilesBuilder()
    runfiles.add(py_executable.venv_python_exe)
    runfiles.add(py_executable.app_runfiles)
    runfiles.add(stage2_bootstrap)

    # todo: add py_runtime runfiles -- weird, it seems to be there, but not
    #       clear where it's coming from.
    # todo: add bazel_site_init to runfiles

    runfiles = runfiles.build(ctx)
    zip_main = _create_zipapp_main_py(ctx, py_runtime, py_executable, stage2_bootstrap)
    inputs = _build_manifest(ctx, manifest, runfiles, zip_main, output)

    output_args = ctx.actions.args()
    output_args.add(output)
    actions_run(
        ctx,
        executable = ctx.attr._zipper,
        arguments = [manifest, output_args],
        inputs = depset(inputs, transitive = [runfiles.files]),
        outputs = [output],
        mnemonic = "PyZipAppCreateZip",
        progress_message = "Reticulating zip archive: %{label} to %{output}",
    )
    return output

def _create_shell_bootstrap(ctx, py_runtime, py_executable, stage2_bootstrap):
    preamble = ctx.actions.declare_file(ctx.label.name + ".preamble.sh")

    bundled_pyexe_path = ""
    external_pyexe_path = ""
    if py_runtime.interpreter_path:
        external_pyexe_path = py_runtime.interpreter_path
    else:
        bundled_pyexe_path = runfiles_root_path(ctx, py_runtime.interpreter.short_path)

    substitutions = {
        "%BUNDLED_PYEXE_PATH%": bundled_pyexe_path,
        "%EXTERNAL_PYEXE_PATH%": external_pyexe_path,
        "%EXTRACT_DIR%": paths.join(
            (ctx.label.repo_name or "_main"),
            ctx.label.package,
            ctx.label.name,
        ),
        "%INTERPRETER_ARGS%": "\n".join(py_executable.interpreter_args),
        "%STAGE2_BOOTSTRAP%": runfiles_root_path(ctx, stage2_bootstrap.short_path),
    }
    ctx.actions.expand_template(
        template = ctx.file._zip_shell_template,
        output = preamble,
        substitutions = substitutions,
        is_executable = True,
    )
    return preamble

def _create_self_executable_zip(ctx, preamble, zip_file):
    pyz = ctx.actions.declare_file(ctx.label.name + ".pyz")
    args = ctx.actions.args()
    args.add(preamble)
    args.add(zip_file)
    args.add(pyz)
    actions_run(
        ctx,
        executable = ctx.attr._exe_zip_maker,
        arguments = [args],
        inputs = [preamble, zip_file],
        outputs = [pyz],
        mnemonic = "PyZipAppCreateExecutableZip",
        progress_message = "Reticulating zipapp: %{label} into %{output}",
    )
    return pyz

def _py_zipapp_binary_impl(ctx):
    py_executable = ctx.attr.binary[PyExecutableInfo]
    py_runtime = ctx.attr.binary[PyRuntimeInfo]
    ## zipapp_toolchain = ctx.toolchains[PY_ZIPAPP_TOOLCHAIN].py_zipapp_toolchain

    stage2_bootstrap = _create_stage2_bootstrap(ctx, py_executable)

    zip_file = _create_zip(ctx, py_runtime, py_executable, stage2_bootstrap)
    if ctx.attr.executable:
        preamble = _create_shell_bootstrap(ctx, py_runtime, py_executable, stage2_bootstrap)
        executable = _create_self_executable_zip(ctx, preamble, zip_file)
    else:
        executable = zip_file

    return [
        DefaultInfo(
            files = depset([executable]),
            runfiles = ctx.runfiles(files = [executable]),
            executable = executable,
        ),
    ]

py_zipapp_binary_rule = rule(
    implementation = _py_zipapp_binary_impl,
    attrs = {
        "binary": attr.label(
            providers = [PyExecutableInfo, PyRuntimeInfo],
            mandatory = True,
        ),
        "deps": attr.label_list(providers = [PyInfo], allow_files = True),
        "executable": attr.bool(default = True),
        "srcs": attr.label_list(allow_files = True),
        "_exe_zip_maker": attr.label(
            cfg = "exec",
            default = "//tools/zipper:exe_zip_maker",
        ),
        "_zip_shell_template": attr.label(
            default = ":zip_shell_template",
            allow_single_file = True,
        ),
        "_zipper": attr.label(
            cfg = "exec",
            default = "//tools/zipper:zipper",
        ),
    },
    executable = True,
    toolchains = [EXEC_TOOLS_TOOLCHAIN_TYPE],
)
