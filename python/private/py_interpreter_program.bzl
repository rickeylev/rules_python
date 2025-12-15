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

"""Internal only bootstrap level binary-like rule."""

load("@bazel_skylib//rules:common_settings.bzl", "BuildSettingInfo")
load("//python/private:sentinel.bzl", "SentinelInfo")
load("//python/private:toolchain_types.bzl", "EXEC_TOOLS_TOOLCHAIN_TYPE")

PyInterpreterProgramInfo = provider(
    doc = "Information about how to run a program with an external interpreter.",
    fields = {
        "env": "dict[str, str] of environment variables to set prior to execution.",
        "interpreter_args": "List of strings; additional args to pass " +
                            "to the interpreter before the main program.",
        "main": "File; the .py file that is the entry point.",
    },
)

def _py_interpreter_program_impl(ctx):
    # Bazel requires the executable file to be an output created by this target.
    # To avoid colliding with the source file (e.g. target=foo, main=foo.py),
    # we append an underscore to the name, but keep the extension so that
    # the original extension is preserved.
    extension = ctx.file.main.extension
    executable_name = "{}_.{}".format(ctx.label.name, extension)
    executable = ctx.actions.declare_file(executable_name)
    ctx.actions.symlink(output = executable, target_file = ctx.file.main)
    execution_requirements = {}
    if BuildSettingInfo in ctx.attr.execution_requirements:
        execution_requirements.update([
            value.split("=", 1)
            for value in ctx.attr.execution_requirements[BuildSettingInfo].value
            if value.strip()
        ])

    return [
        DefaultInfo(
            executable = executable,
            files = depset([executable]),
            runfiles = ctx.runfiles(files = [
                executable,
            ]),
        ),
        PyInterpreterProgramInfo(
            env = ctx.attr.env,
            interpreter_args = ctx.attr.interpreter_args,
            main = ctx.file.main,
        ),
        testing.ExecutionInfo(
            requirements = execution_requirements,
        ),
    ]

py_interpreter_program = rule(
    doc = """
Binary-like rule that doesn't require a toolchain because its part of
implementing build tools for the toolchain. This rule expects the Python
interprter to be externally provided.

To run a `py_interpreter_program` as an action, pass it as a tool that is
used by the actual interpreter executable. This ensures its runfiles are
setup. Also pass along any interpreter args, environment, and requirements.

```starlark
ctx.actions.run(
    executable = <python interpreter executable>,
    args = (
        target[PyInterpreterProgramInfo].interpreter_args +
        [target[DefaultInfo].files_to_run.executable]
    ),
    tools = target[DefaultInfo].files_to_run,
    env = target[PyInterpreterProgramInfo].env,
    execution_requirements = target[testing.ExecutionInfo].requirements,
)
```

""",
    implementation = _py_interpreter_program_impl,
    attrs = {
        "env": attr.string_dict(
            doc = "Environment variables that should set prior to running.",
        ),
        "execution_requirements": attr.label(
            default = "//python:none",
            doc = "Execution requirements to set when running it as an action",
            providers = [[BuildSettingInfo], [SentinelInfo]],
        ),
        "interpreter_args": attr.string_list(
            doc = "Args that should be passed to the interpreter.",
        ),
        "main": attr.label(
            doc = "The entry point Python file.",
            allow_single_file = True,
        ),
    },
    # This is set to False because this isn't a binary/executable in the usual
    # Bazel sense (even though it sets DefaultInfo.files_to_run). It just holds
    # information so that a caller can construct how to execute it correctly.
    executable = False,
)

def actions_run(
        ctx,
        *,
        executable,
        toolchain = None,
        **kwargs):
    """Runs a tool as an action, supporting py_interpreter_program targets.

    This is wrapper around `ctx.actions.run()` that sets some useful defaults,
    supports handling `py_interpreter_program` targets, and some other features
    to let the target being run influence the action invocation.

    Args:
        ctx: The rule context. The rule must have the
            `//python:exec_tools_toolchain_type` toolchain available.
        executable: The executable to run. This can be a target that provides
            `PyInterpreterProgramInfo` or a regular executable target. If it
            provides `testing.ExecutionInfo`, the requirements will be added to
            the execution requirements.
        toolchain: The toolchain type to use. Must be None or
            `//python:exec_tools_toolchain_type`.
        **kwargs: Additional arguments to pass to `ctx.actions.run()`.
            `mnemonic` and `progress_message` are required.
    """
    mnemonic = kwargs.pop("mnemonic", None)
    if not mnemonic:
        fail("actions_run: missing required argument 'mnemonic'")

    progress_message = kwargs.pop("progress_message", None)
    if not progress_message:
        fail("actions_run: missing required argument 'progress_message'")

    tools = kwargs.pop("tools", None)
    tools = list(tools) if tools else []
    arguments = kwargs.pop("arguments", [])

    action_arguments = []
    action_env = {
        "PYTHONHASHSEED": "0",  # Helps avoid non-deterministic behavior
        "PYTHONNOUSERSITE": "1",  # Helps avoid non-deterministic behavior
        "PYTHONSAFEPATH": "1",  # Helps avoid incorrect import issues
    }
    default_info = executable[DefaultInfo]
    if PyInterpreterProgramInfo in executable:
        if toolchain and toolchain != EXEC_TOOLS_TOOLCHAIN_TYPE:
            fail(("Action {}: tool {} provides PyInterpreterProgramInfo, which " +
                  "requires the `toolchain` arg be " +
                  "None or {}, got: {}").format(
                mnemonic,
                executable,
                EXEC_TOOLS_TOOLCHAIN_TYPE,
                toolchain,
            ))
        exec_tools = ctx.toolchains[EXEC_TOOLS_TOOLCHAIN_TYPE].exec_tools
        action_exe = exec_tools.exec_interpreter[DefaultInfo].files_to_run

        program_info = executable[PyInterpreterProgramInfo]

        interpreter_args = ctx.actions.args()
        interpreter_args.add_all(program_info.interpreter_args)
        interpreter_args.add(default_info.files_to_run.executable)
        action_arguments.append(interpreter_args)

        action_env.update(program_info.env)
        tools.append(default_info.files_to_run)
        toolchain = EXEC_TOOLS_TOOLCHAIN_TYPE
    else:
        action_exe = executable[DefaultInfo].files_to_run

    execution_requirements = {}
    if testing.ExecutionInfo in executable:
        execution_requirements.update(executable[testing.ExecutionInfo].requirements)

    # Give precedence to caller's execution requirements.
    execution_requirements.update(kwargs.pop("execution_requirements", None) or {})

    # Give precedence to caller's env.
    action_env.update(kwargs.pop("env", None) or {})
    action_arguments.extend(arguments)
    ctx.actions.run(
        executable = action_exe,
        arguments = action_arguments,
        tools = tools,
        env = action_env,
        execution_requirements = execution_requirements,
        toolchain = toolchain,
        mnemonic = mnemonic,
        progress_message = progress_message,
        **kwargs
    )
