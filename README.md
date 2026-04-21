# Python Rules for Bazel

[![Build status](https://badge.buildkite.com/0bcfe58b6f5741aacb09b12485969ba7a1205955a45b53e854.svg?branch=main)](https://buildkite.com/bazel/rules-python-python)

## Overview

This repository is the home of the core Python rules -- `py_library`,
`py_binary`, `py_test`, and related symbols that provide the basis for Python
support in Bazel. It also contains package installation rules for integrating
with PyPI and other indices.

Documentation for rules_python is at <https://rules-python.readthedocs.io> and in the
[Bazel Build Encyclopedia](https://docs.bazel.build/versions/master/be/python.html).

Examples live in the [examples](examples) directory.

The core rules are stable. Their implementation is subject to Bazel's
[backward compatibility policy](https://docs.bazel.build/versions/master/backward-compatibility.html).
This repository aims to follow [semantic versioning](https://semver.org).

The Bazel community maintains this repository. Neither Google nor the Bazel team
provides support for the code. However, this repository is part of the test
suite used to vet new Bazel releases. See [How to contribute](CONTRIBUTING.md)
page for information on our development workflow.

## Design

* Supported platforms: we strive to support platforms we have CI for, but
  platforms without an interested maintainer receive less backward
  compatibility guarantees and require the community to contribute features and
  fixes. See
  [support policy](https://rules-python.readthedocs.io/en/latest/support.html)
  for more information.
* External dependency management is currently based on locked `requirements.txt`
  files. Resolution of packages to URLs is handled by Starlark during the Bzlmod
  phase, respecting custom PyPI index settings, and cached in
  `MODULE.bazel.lock`. Thus, requirements files are as cross-platform compatible
  as their environment marker lines and Bazel configuration allows. Using `uv`
  is recommended for generating fully locked `requirements.txt` files that are
  as cross-platform as the uv configuration allows. The
  [uv `lock()` rule](https://rules-python.readthedocs.io/en/latest/api/rules_python/python/uv/lock.html)
  is provided for using `uv` to create hermeticly built lock files.
* The `py_binary` and `py_test` rules are designed to scale to large programs
  in large, complex, monorepos with multiple independently developed
  sub-projects within it. This means they try to minimize network and file
  operations so that large programs can scale performantly. Notably,
  dependencies are installed once and symlinks used to minimize venv creation
  cost instead of each target copying the full venv.
* As of 2.0, minimal virtual environments are created for each binary/test by
  default. Full venv creation is enabled by setting
  [`--venv_site_packages=yes`](https://rules-python.readthedocs.io/en/latest/api/rules_python/python/config_settings/#venvs_site_packages),
  which will become the default in a future release.
* Support for standards - we strive to first implement any standards needed
  within `rules_python` and this has resulted in a few PEPs supported within
  pure starlark - PEP440, PEP509.

Common misconceptions:
* "`rules_python` has to keep backwards compatibility with Google's internal
  `google3` codebase." While the project originated from Google, it is now a
  separate project run as part of the Bazel Contrib branch of the Linux
  Foundation. It has no requirement to maintain compatibility with Google's
  internal codebase.
* "`rules_python` is not caching pip downloads." As of 2.0, Bazel's
  downloader, not pip, is used by default. This provides Bazel's usual
  repository caching mechanisms, which are transparent and scalable.

## Documentation

For detailed documentation, see <https://rules-python.readthedocs.io>

## Bzlmod support

See [Bzlmod support](BZLMOD_SUPPORT.md) for more details.
