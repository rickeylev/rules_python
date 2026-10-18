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
