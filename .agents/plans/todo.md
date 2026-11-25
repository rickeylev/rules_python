# Native Bootstrap Method Todo

- [x] Survey bootstrap flags, `py_runtime`, `py_executable.bzl`, and
      `stage1_bootstrap_template.sh`.
- [x] Write `.agents/plans/native-bootstrap-method.md` with
      `"Discarded alternatives"` and `"Open questions"`.
- [x] Create `.agents/plans/learnings.md` and `.agents/plans/todo.md`.
- [x] Add `NATIVE = "native"` to `BootstrapImplFlag` in
      `python/private/flags.bzl`.
- [x] Implement the native stage-1 bootstrap template and embedder in Rust under
      `crates/stage1_native_bootstrap/` and wire `crates/BUILD.bazel`.
- [x] Expose `stage1_native_bootstrap_template`,
      `stage1_native_bootstrap_embedder`, `native_bootstrap_template`, and
      `is_native_bootstrap_enabled` in `python/private/BUILD.bazel`.
- [x] Update `python/private/py_executable.bzl` to support
      `BootstrapImplFlag.NATIVE`, embed target metadata into the stage-1 binary,
      and remain compatible with `rules_rust_stub` in WORKSPACE mode.
- [x] Add `rules_testing` analysis tests in
      `tests/base_rules/py_executable_base_tests.bzl` and
      `tests/py_runtime/py_runtime_tests.bzl`.
- [x] Add Rust unit tests (`rust_test`) in `tests/stage1_native_bootstrap/` and
      execution tests (including prebuilt bootstrap binary test) in
      `tests/bootstrap_impls/`.
- [x] Update documentation in
      `docs/api/rules_python/python/config_settings/index.md`, rule docstrings,
      and `news/native-bootstrap-impl.added.md`.
- [x] Verify all tests pass with `bazel test --config=fast-tests` and docs build
      with `bazel build //docs:docs`.
- [x] Commit deliverables and push to `origin`
      (`HEAD:add_native_bootstrap_method`).
- [ ] **[IMPORTANT]** Add macOS Apple Silicon (`arm64`) Mach-O ad-hoc re-signing
      (or `codesign -s -` / `apple-codesign`) to `lib.rs` for
      `--bootstrap_impl=native`: on Apple Silicon, macOS `amfi` kills
      (`SIGKILL`) Mach-O binaries whose mapped `.data` / `__TEXT` bytes are
      modified after linking without updating `LC_CODE_SIGNATURE`.
- [ ] Shrink `stage1_native_bootstrap_template` binary size by switching Linux
      builds to `musl` static linking, stripping symbols, and enabling
      `opt-level = "z"` and `panic = "abort"` to cut per-`py_binary` disk size
      from megabytes down to tens/hundreds of kilobytes.
- [ ] Place `G_BOOTSTRAP_CFG` in a read-only section (`.rodata` /
      `__TEXT,__const`), patched on disk and read via volatile pointer at
      runtime, to avoid dirtying a writable `.data` page per process.
- [ ] On Windows, pass the resolved `GetFinalPathNameByHandleW` interpreter as
      `lpApplicationName` while setting `__PYVENV_LAUNCHER__` and `argv[0]` to
      the venv interpreter path, and attach a Job Object
      (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) for DLL loading and orphan child
      cleanup.
- [x] Fold `native_bootstrap_embedder` into `shrubbery` (`exec_compatible_with`)
      and dispatch `PyEmbedNativeBootstrap` through `get_shrubbery_tool`.
- [ ] Wire prebuilt `stage1_native_bootstrap_template` binaries into a
      `target_compatible_with` toolchain (`dev/release_artifacts` and
      `@rules_python_tools`) so any exec platform can cross-build launchers for
      any target platform without `rules_rust` or manually setting
      `--//python/private:native_bootstrap_template`.
