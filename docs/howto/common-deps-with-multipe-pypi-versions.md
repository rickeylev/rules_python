# How to use a common set of dependencies with multiple PyPI versions

The `pypi` flag allows you to build and test a `py_binary` or `py_test` target
against different sets of dependencies from different PyPI repositories. This is
useful when you have multiple PyPI repositories and you want to ensure that your
package works correctly with the dependencies from each of them.

The `pypi` flag is a `string_flag` that can be set on the command line when you
build or test a target. The value of the flag is then used to select a specific
configuration for the target. Each configuration can have its own set of
dependencies.

For example, to test a `py_test` target against two different PyPI
repositories, `alpha` and `beta`.

First, you need to define a `config_setting` for each PyPI repository in your
`BUILD` file:

```bazel
config_setting(
    name = "pypi_alpha",
    flag_values = {"//python/config_settings:pypi": "alpha"},
)

config_setting(
    name = "pypi_beta",
    flag_values = {"//python/config_settings:pypi": "beta"},
)
```

Next, you can use these `config_setting`s in a `select` statement to specify
the dependencies for each configuration:

```bazel
py_test(
    name = "my_test",
    srcs = ["my_test.py"],
    pypi = "alpha",  # Default value
    deps = select({
        ":pypi_alpha": ["@pypi_alpha//my_dep"],
        ":pypi_beta": ["@pypi_beta//my_dep"],
    }),
)
```

Now, you can run the test against the `alpha` repository with the following
command:

```bash
bazel test //:my_test
```

To run the test against the `beta` repository, you can set the `pypi` flag on
the command line:

```bash
bazel test //:my_test --//python/config_settings:pypi=beta
```

This will build and run the `my_test` target with the dependencies from the
`beta` repository.