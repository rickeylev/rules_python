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
   - Users enable the native bootstrap method via
     `--//python/config_settings:bootstrap_impl=native` (`BootstrapImplFlag` in
     `python/private/flags.bzl`). `//python/config_settings:bootstrap_impl`
     derives its allowed values from `BootstrapImplFlag.__members__.values()`,
     so `native` automatically integrates with existing `config_settings`
     transitions, `py_reconfig_binary`, and `py_reconfig_test`.
   - Add `config_setting(name = "is_native_bootstrap_enabled", ...)` in
     `python/private/BUILD.bazel`.
   - Add `label_flag(name = "native_bootstrap_template", ...)` defaulting to
     `:stage1_native_bootstrap_template`, and select it in
     `//python/private:bootstrap_template` when `:is_native_bootstrap_enabled`
     matches.
   - *Why `//python/private:native_bootstrap_template` (`label_flag`) is
     needed in addition to `py_runtime(bootstrap_template = ...)`*:
     - Built-in hermetic runtimes registered via `python.toolchain()` (Bzlmod)
       or `python_register_toolchains()` (WORKSPACE) generate `py_runtime`
       targets internally (`hermetic_runtime_repo_setup.bzl`) without exposing
       `bootstrap_template` on the toolchain extension; they all use the default
       `//python/private:bootstrap_template`.
     - Because `@rules_rust` is a `dev_dependency = True` in `rules_python`'s
       `MODULE.bazel`, downstream Bzlmod modules and WORKSPACE builds cannot
       compile `//crates/stage1_native_bootstrap` from Rust source inside
       `@rules_python`.
     - Setting `py_runtime(bootstrap_template = <file>)` directly on a runtime
       replaces `//python/private:bootstrap_template`'s `select()` for all
       `--bootstrap_impl` modes unless the caller writes a custom `select()`.
     - The `native_bootstrap_template` `label_flag` is only selected when
       `--bootstrap_impl=native` is active and allows supplying a prebuilt
       native bootstrap binary across all built-in hermetic toolchains (and
       per-target `config_settings` transitions) without redefining custom
       `py_runtime`, `py_runtime_pair`, and `toolchain` targets.
2. **Native Stage-1 Template (`crates/stage1_native_bootstrap/`)**:
   - Compile a Rust stage-1 binary (`stage1_native_bootstrap_template`) with a
     4 KiB (`4096`-byte) static configuration buffer in `.data` prefixed by a
     32-byte magic sentinel (`\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\0`).
   - Support an EOF overflow trailer (`\0RPY_NATIVE_EOF_CFG_V1\0\0`) if target
     metadata exceeds the 4 KiB in-binary buffer capacity.
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
   - Compile a Rust embedder binary (`stage1_native_bootstrap_embedder`) — and
     allow an unconfigured `stage1_native_bootstrap_template` binary itself to
     run the embedder when invoked with `--template` — accepting `--template`,
     `--output`, and target bootstrap metadata flags.
   - Copy the input template binary (either compiled from source or provided as
     a prebuilt binary via `py_runtime(bootstrap_template = ...)` or
     `--//python/private:native_bootstrap_template`), locate the magic sentinel,
     and patch the NUL-delimited `key=value` payload in-place without changing
     ELF/Mach-O/PE section offsets.
   - Fail at build time with a descriptive error if the input `template` does
     not contain the 32-byte `CFG_MAGIC` sentinel (e.g., if a text template was
     mistakenly provided under `--bootstrap_impl=native`).
   - If the payload exceeds the fixed 4 KiB buffer (`> 4044` bytes), mark
     `overflow=1` in the buffer and append an EOF trailer block.
4. **Rule Integration (`python/private/py_executable.bzl`)**:
   - Gate `//crates/stage1_native_bootstrap:...` aliases in
     `python/private/BUILD.bazel` with `IS_BZLMOD_ROOT_MODULE`
     (`python/private/bzlmod_enabled.bzl`) so downstream Bzlmod modules (where
     `rules_rust` is a `dev_dependency = True`) and WORKSPACE builds do not load
     `//crates/...`.
   - Add `_native_bootstrap_embedder` (`cfg = "exec"`, `allow_files = True`) to
     `EXECUTABLE_ATTRS`, falling back to executing `template` itself when no
     separate embedder executable is present.
   - Treat `BootstrapImplFlag.NATIVE` alongside `BootstrapImplFlag.SCRIPT` for
     build-time venv creation.
   - In `_create_stage1_bootstrap`, run `PyEmbedNativeBootstrap` when
     `BootstrapImplFlag.get_value(ctx) == BootstrapImplFlag.NATIVE`.

## How

### Binary Configuration Layout and Buffer Size Justification

The stage-1 binary embeds a 4 KiB (`4096`-byte) array in `.data`:

```text
Offset 0..31:     "\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\0" (32-byte magic)
Offset 32..35:    uint32_le payload_length
Offset 36..4079:  NUL-terminated "key=value\0" entries, terminated by "\0"
Offset 4080..4095: 16-byte zero footer verified by find_magic_offset
```

**Why 4 KiB (`4096` bytes) was chosen**:
- Because `G_BOOTSTRAP_CFG` is initialized with the non-zero 32-byte
  `CFG_MAGIC` prefix, the linker places the entire array in `.data`
  (`SHT_PROGBITS` on ELF) rather than `.bss` (`SHT_NOBITS`). Every byte in
  `CFG_BUFFER_SIZE` is physically stored on disk in the template binary and in
  every `py_binary` / `py_test` executable produced from it.
- A typical target's serialized `key=value` payload (`stage2_bootstrap`,
  `python_binary`, `python_binary_actual`, `venv_rel_site_packages`,
  `workspace_name`, `target`, three `"0"`/`"1"` flags, and `interpreter_args`)
  is ~300–600 bytes; even when `--venvs_use_declare_symlink=no` embeds
  interpreter `runtime_venv_symlinks`, the payload is ~800–1,800 bytes.
- Choosing 4 KiB (`4096` bytes) matches a single OS memory page (`0x1000`
  alignment), provides `4044` bytes of usable in-place payload capacity
  (`4096 - 36 header - 16 footer`, >2x–6x typical payloads) while costing only
  one 4 KiB page on disk per binary (versus 64 KiB), and transparently falls
  back to the EOF overflow trailer for targets with unusually large
  `runtime_venv_symlinks` or `interpreter_args`.

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

### How `py_executable` Dispatches Between Text Expansion and Binary Embedding

During Bazel's analysis phase, `py_executable` only has a `File` handle for
`runtime.bootstrap_template` (`PyRuntimeInfo.bootstrap_template`) and cannot
inspect the file's bytes to determine whether it is a text template or a native
binary.

Instead, `_create_stage1_bootstrap` in `python/private/py_executable.bzl`
dispatches based on `--//python/config_settings:bootstrap_impl`
(`BootstrapImplFlag.get_value(ctx)`):
- When `bootstrap_impl` is `system_python` or `script`, `py_executable` treats
  `runtime.bootstrap_template` as a text file and registers
  `ctx.actions.expand_template`.
- When `bootstrap_impl` is `native`, `py_executable` treats
  `runtime.bootstrap_template` as a native binary template and registers the
  `PyEmbedNativeBootstrap` action (`stage1_native_bootstrap_embedder`).

Consequently, `PyRuntimeInfo.bootstrap_template` must match the active
`bootstrap_impl` mode (just as `system_python` and `script` already require
different text templates). The default `//python/private:bootstrap_template`
alias handles this automatically via `select()` on `bootstrap_impl`; custom
`py_runtime` targets that set `bootstrap_template` directly must either provide
a binary template when `--bootstrap_impl=native` is enabled or use a `select()`
on `bootstrap_impl`. If a text file or binary lacking `CFG_MAGIC` is passed to
`PyEmbedNativeBootstrap`, the embedder fails the build action with an explicit
error.

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
3. **`ctx.actions.expand_template` on placeholders inside a binary**:
   - Discarded because `ctx.actions.expand_template` is intended for text files,
     not binary files (Bazel decodes the template as text, which corrupts
     non-text binary byte sequences), and replacing variable-length `%...%`
     strings inside `.rodata` shifts subsequent bytes in the binary and corrupts
     ELF `e_shoff`, `PT_LOAD` `p_offset`, and Mach-O segment offsets.
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

1. **Should prebuilt bootstrap template binaries be supplied via
   `--//python/private:native_bootstrap_template` (`label_flag`),
   `py_runtime(bootstrap_template = ...)`, or a toolchain?**
   - *Decision*: Support both `py_runtime(bootstrap_template = ...)` and
     `--//python/private:native_bootstrap_template` (`label_flag`). Because
     `python.toolchain()` does not expose `py_runtime.bootstrap_template` on
     built-in hermetic runtimes and `@rules_rust` is a `dev_dependency = True`
     in `MODULE.bazel`, the `label_flag` provides a way to supply a prebuilt
     native template binary without redefining custom `py_runtime` and
     `toolchain` targets, while `py_runtime(bootstrap_template = ...)` continues
     to work for custom runtimes.
2. **How should payloads larger than the 4 KiB in-binary buffer be handled?**
   - *Decision*: Reserve a 4 KiB (`4096`-byte, 1 OS page) buffer in `.data`
     (sufficient for standard targets) and automatically fall back to appending
     an EOF trailer (`\0RPY_NATIVE_EOF_CFG_V1\0\0`) when the serialized metadata
     exceeds `4044` bytes.
