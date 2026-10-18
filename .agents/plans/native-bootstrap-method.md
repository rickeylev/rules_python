# Native Executable Bootstrap Method

## Why

`rules_python` supports two stage-1 bootstrap implementations via
`--//python/config_settings:bootstrap_impl`:

1. `system_python` (`python_bootstrap_template.txt`): Requires a host Python
   interpreter on `PATH` or at a fixed path before finding the hermetic runtime
   in runfiles.
2. `script` (`stage1_bootstrap_template.sh`): Uses `/usr/bin/env bash` to find
   runfiles, materialize runtime virtualenvs or zip archives, and exec the
   target interpreter with `stage2_bootstrap_template.py`.

Both approaches depend on external interpreters (`python3` or `bash`, `env`,
`dirname`, `readlink`, `mktemp`) at startup and spawn multiple subshells. A
native stage-1 executable (`--//python/config_settings:bootstrap_impl=native`)
eliminates the shell dependency, reduces startup overhead to direct OS
syscalls, and allows a prebuilt binary to serve as the bootstrap template across
all `py_binary` and `py_test` targets without compiling native code (C, C++, or
Rust) per target.

## What

1. **Flag and Template Selection**:
   - Add `NATIVE = "native"` to `BootstrapImplFlag` in
     `python/private/flags.bzl`. `//python/config_settings:bootstrap_impl`
     derives allowed values from `BootstrapImplFlag.__members__.values()`.
   - Add `config_setting(name = "is_native_bootstrap_enabled", ...)` in
     `python/private/BUILD.bazel`.
   - Add `label_flag(name = "native_bootstrap_template", ...)` defaulting to
     `:stage1_native_bootstrap_template`, and select it in
     `//python/private:bootstrap_template` when `:is_native_bootstrap_enabled`
     matches.
2. **Native Stage-1 Template (`crates/stage1_native_bootstrap/`)**:
   - Compile a Rust stage-1 binary (`stage1_native_bootstrap_template`) with a
     64 KiB static configuration buffer in `.data` prefixed by a 32-byte magic
     sentinel (`\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\0`).
   - Support an EOF overflow trailer (`\0RPY_NATIVE_EOF_CFG_V1\0\0`) if target
     metadata exceeds the in-binary buffer capacity.
   - Implement full stage-1 runtime parity with `stage1_bootstrap_template.sh`:
     - Runfiles discovery from `RUNFILES_DIR` and `RUNFILES_MANIFEST_FILE`
       validated by checking `<maybe_root>/<stage2_bootstrap>`, falling back to
       `argv[0]` symlink-hop resolution (`<stub>.runfiles` and `*.runfiles/*`).
     - `RULES_PYTHON_TESTING_TELL_RUNFILES_ROOT` export to
       `RULES_PYTHON_TESTING_RUNFILES_ROOT`.
     - Self-extracting zip execution (`is_zipfile=1`) via temporary directory
       creation, `unzip`, venv interpreter symlink creation, signal forwarding
       (`SIGTERM`, `SIGINT`, `SIGQUIT`, `SIGHUP` while preserving `SIG_IGN`),
       and synchronous temporary directory cleanup before exit.
     - Runtime virtualenv creation (`recreate_venv_at_runtime=1`) in
       `RULES_PYTHON_EXTRACT_ROOT` or a temporary directory, supporting
       `resolve_python_binary_at_runtime=1`, `pyvenv.cfg`, and
       `runtime_venv_symlinks`.
     - Interpreter path resolution (`/*`, `*/*` under `RUNFILES_DIR`, or `PATH`
       lookup).
     - `PYTHONSAFEPATH=1` default when `PYTHONSAFEPATH` is unset in the
       environment (preserving `PYTHONSAFEPATH=""`), whitespace splitting and
       unsetting of `RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS`, and `execv`
       into `<python_exe>` with target `interpreter_args` and
       `<runfiles_dir>/<stage2_bootstrap>`.
3. **Prebuilt Binary Embedder (`crates/stage1_native_bootstrap/`)**:
   - Compile a Rust embedder binary (`stage1_native_bootstrap_embedder`) that
     accepts `--template`, `--output`, and target bootstrap metadata flags.
   - Copy the input template binary (either compiled from source or provided as
     a prebuilt binary via `py_runtime(bootstrap_template = ...)` or
     `--//python/private:native_bootstrap_template`), locate the magic sentinel,
     and patch the NUL-delimited `key=value` payload in-place without changing
     ELF/Mach-O/PE section offsets.
   - If the payload exceeds the fixed buffer, mark `overflow=1` in the buffer
     and append an EOF trailer block.
   - If a custom text template without the binary magic sentinel is supplied,
     fall back to expanding `%...%` placeholders so custom text templates remain
     compatible.
4. **Rule Integration (`python/private/py_executable.bzl`)**:
   - Add `_native_bootstrap_embedder` (`cfg = "exec"`, `allow_files = True`, and
     gated with `select()` on `//python/private:is_native_bootstrap_enabled` so
     WORKSPACE builds using `tests/modules/rules_rust_stub` do not fail
     analysis) to `EXECUTABLE_ATTRS`.
   - Treat `BootstrapImplFlag.NATIVE` alongside `BootstrapImplFlag.SCRIPT` for
     build-time venv creation.
   - In `_create_stage1_bootstrap`, run `PyEmbedNativeBootstrap` when
     `BootstrapImplFlag.get_value(ctx) == BootstrapImplFlag.NATIVE`.

## How

### Binary Configuration Layout

The stage-1 binary embeds a 65536-byte array in `.data`:

```text
Offset 0..31:   "\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\0" (32-byte magic)
Offset 32..35:  uint32_le payload_length
Offset 36..N:   NUL-terminated "key=value\0" entries, terminated by "\0"
```

Keys embedded by `_create_stage1_bootstrap`:
- `stage2_bootstrap`: Runfiles-relative path to stage-2 `.py` script.
- `python_binary`: Runfiles-relative path to venv interpreter.
- `python_binary_actual`: Underlying runtime interpreter path.
- `is_zipfile`: `"1"` or `"0"`.
- `recreate_venv_at_runtime`: `"1"` or `"0"`.
- `resolve_python_binary_at_runtime`: `"1"` or `"0"`.
- `venv_rel_site_packages`: Venv-relative `site-packages` path.
- `workspace_name`: Target workspace name.
- `target`: Target label string.
- `interpreter_args`: Newline-delimited target interpreter arguments.
- `runtime_venv_symlinks`: Newline-delimited `<venv_rel>|<runfiles_rel>` pairs.

Because the embedder overwrites bytes inside the existing `.data` array without
changing file size, ELF section headers, `PT_LOAD` offsets, and self-extracting
zip preambles (`exe_zip_maker`) work without binary relinking.

## Discarded alternatives

1. **Compiling C/C++/Rust source per `py_binary` / `py_test` target**:
   - Discarded because invoking a native compiler and linker per Python target
     adds compile/link overhead to every binary and prevents using a prebuilt
     bootstrap binary without a target native toolchain.
2. **Implementing the native bootstrap in C under `python/private/`**:
   - Discarded in favor of Rust under `crates/` to match `rules_python`'s
     existing native tool architecture (`crates/exe_zip_maker`), benefit from
     memory safety and `rust_test` unit testing, and avoid manual C buffer
     management.
3. **Naive `ctx.actions.expand_template` on placeholders inside `.rodata`**:
   - Discarded because replacing variable-length `%...%` strings inside
     `.rodata` shifts subsequent bytes in the binary and corrupts ELF `e_shoff`,
     `PT_LOAD` `p_offset`, and Mach-O segment offsets.
4. **Unmodified shared binary with an external config file in runfiles**:
   - Discarded because stage 1 must know `<stage2_bootstrap>` before trusting an
     inherited `RUNFILES_DIR` (when an outer binary invokes an inner binary that
     is not a data dependency), and `--build_python_zip` requires a single
     self-contained file.
5. **Detached `sleep 1` polling background process for zip cleanup**:
   - Discarded in favor of a parent process that forwards signals (`SIGTERM`,
     `SIGINT`, `SIGQUIT`, `SIGHUP`), waits for the child Python process, and
     synchronously removes the temporary directory before exiting. This avoids
     orphan `sleep` processes, guarantees immediate cleanup upon process exit,
     and preserves exact exit status codes.
6. **Linking `libpython` directly into the stage-1 binary**:
   - Discarded because stage 1 must work across arbitrary Python versions,
     platform runtimes, and build-time `.venv` symlink trees.

## Open questions

1. **How should users enable the native bootstrap method?**
   - *Decision*: Add `native` to `--//python/config_settings:bootstrap_impl`
     (`BootstrapImplFlag.NATIVE`). This integrates with existing flag
     transitions (`config_settings`, `py_reconfig_binary`, `py_reconfig_test`)
     and `//python/private:bootstrap_template` selection.
2. **How should prebuilt bootstrap template binaries be supplied?**
   - *Decision*: Support both `py_runtime(bootstrap_template = ...)` and a
     dedicated `--//python/private:native_bootstrap_template` `label_flag`
     defaulting to `//python/private:stage1_native_bootstrap_template`. Either
     mechanism accepts any prebuilt binary containing the configuration buffer.
3. **How should payloads larger than the fixed in-binary buffer be handled?**
   - *Decision*: Reserve a 64 KiB buffer in `.data` (sufficient for almost all
     targets) and support an automatic EOF trailer overflow fallback when the
     serialized metadata exceeds 64 KiB.
