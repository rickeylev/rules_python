# Agent Configuration Learnings

1. **Dynamic Flag Values in `python/config_settings/BUILD.bazel`**:
   - `//python/config_settings:bootstrap_impl` populates `values` dynamically
     from `sorted(BootstrapImplFlag.__members__.values())`. Adding a member to
     `BootstrapImplFlag` in `python/private/flags.bzl` registers the CLI value
     without editing `python/config_settings/BUILD.bazel`.
2. **Runfiles Discovery Validation Order**:
   - Stage-1 bootstraps must validate `RUNFILES_DIR` and
     `RUNFILES_MANIFEST_FILE` by checking that
     `<maybe_root>/<stage2_bootstrap>` exists before trusting the environment
     variable. Otherwise, an outer binary invoking an independent inner binary
     leaks the outer runfiles root.
3. **`PYTHONSAFEPATH` Empty-String Opt-Out**:
   - Stage-1 bootstraps must distinguish an unset `PYTHONSAFEPATH`
     (`std::env::var_os("PYTHONSAFEPATH").is_none()`, which defaults to `"1"`)
     from an explicitly empty `PYTHONSAFEPATH=""` (which preserves the user
     opt-out).
4. **Synchronous Temporary Directory Cleanup and Signal Forwarding**:
   - Zip (`--build_python_zip`) and runtime-venv
     (`--venvs_use_declare_symlink=no`) tests assert that temporary directories
     are deleted immediately when the process exits and that `SIGHUP` ignore
     dispositions (`SIG_IGN`) and `SIGTERM` exit codes (`0` when handled, `143`
     when unhandled) are preserved. Spawning the interpreter, forwarding
     non-ignored signals, cleaning up after `wait`, and exiting with the
     child's status satisfies both invariants deterministically.
5. **Starlark Analysis Test Ordering**:
   - `_tests` in `tests/base_rules/py_executable_base_tests.bzl` must remain in
     strict alphabetical order.
6. **Transition Flag Registration and `rules_testing` Label Resolution**:
   - New build flags transitioned on by `py_binary`/`py_test`/`py_reconfig_*`
     must be listed in `python/private/common_labels.bzl` and
     `python/private/transition_labels.bzl`.
   - When overriding a `label_flag` inside `@rules_testing`'s
     `analysis_test(config_settings = ...)`, wrap the target string with
     `str(Label("//..."))` so the label resolves in the root repository rather
     than `@@rules_testing+`.
7. **WORKSPACE `rules_rust_stub` Compatibility for `crates/` Targets**:
   - In WORKSPACE mode, `@rules_rust//rust:defs.bzl` maps to
     `tests/modules/rules_rust_stub/rust/defs.bzl`, where `rust_binary` expands
     to a non-executable `native.filegroup(name = name, tags = ["manual"])`.
   - Rule attributes referencing a `rust_binary` (such as
     `_native_bootstrap_embedder` in `python/private/py_executable.bzl`) must
     gate the label with `select()` on
     `//python/private:is_native_bootstrap_enabled` and use `allow_files = True,
     cfg = "exec"` so non-native bootstrap builds in WORKSPACE mode do not fail
     attribute analysis.
