##&& .\bazel-bin\tests\bootstrap_impls\_run_binary_bootstrap_script_zip_no_test_bin.ps1.runfiles\_main\tests\bootstrap_impls\_run_binary_bootstrap_script_zip_no_test_bin.ps1


bazel build `
  --spawn_strategy=local `
  --verbose_failures `
  --subcommands=pretty_print `
  --@rules_python//python/config_settings:bootstrap_impl=script `
  //tests/bootstrap_impls:_run_binary_bootstrap_script_zip_no_test_bin `
  && .\bazel-bin\tests\bootstrap_impls\_run_binary_bootstrap_script_zip_no_test_bin.ps1
