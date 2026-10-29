"""Tests for uv_lock_to_requirements."""

load("@rules_testing//lib:test_suite.bzl", "test_suite")
load("//python/private/pypi:uv_lock_to_requirements.bzl", "uv_lock_to_requirements")  # buildifier: disable=bzl-visibility

_tests = []

def _test_empty(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[]}"""))
    env.expect.that_str(got).equals("")

_tests.append(_test_empty)

def _test_simple_package(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo==0.0.1 \\
    --hash=sha256:deadbeef
""")

_tests.append(_test_simple_package)

def _test_package_with_markers(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"resolution-markers":["python_full_version < '3.10'"],"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo==0.0.1 ; python_full_version < '3.10' \\
    --hash=sha256:deadbeef
""")

_tests.append(_test_package_with_markers)

def _test_package_with_multiple_markers(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"resolution-markers":["python_full_version == '3.10.*'","python_full_version >= '3.14'"],"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo==0.0.1 ; python_full_version == '3.10.*' or python_full_version >= '3.14' \\
    --hash=sha256:deadbeef
""")

_tests.append(_test_package_with_multiple_markers)

def _test_package_with_deps(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"bar","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:baadbeef","url":"https://example.org/bar.whl"}]},
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"bar"}],"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
bar==0.0.1 \\
    --hash=sha256:baadbeef
    # via foo

foo==0.0.1 \\
    --hash=sha256:deadbeef
""")

_tests.append(_test_package_with_deps)

def _test_package_with_optional_deps(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"bar","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:baadbeef","url":"https://example.org/bar.whl"}]},
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"optional-dependencies":{"extra1":[{"name":"bar"}]},"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
bar==0.0.1 \\
    --hash=sha256:baadbeef
    # via foo

foo[extra1]==0.0.1 \\
    --hash=sha256:deadbeef
""")

_tests.append(_test_package_with_optional_deps)

def _test_self_edge_excluded(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"pydantic","version":"2.0.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"pydantic","extra":["email"]}],"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/pydantic.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
pydantic==2.0.0 \\
    --hash=sha256:deadbeef
""")

_tests.append(_test_self_edge_excluded)

def _test_multiple_dependents(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"common","version":"1.0.0","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:aaaa","url":"https://example.org/common.whl"}]},
        {"name":"pkg_a","version":"0.1.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"common"}],"wheels":[{"hash":"sha256:bbbb","url":"https://example.org/a.whl"}]},
        {"name":"pkg_b","version":"0.2.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"common"}],"wheels":[{"hash":"sha256:cccc","url":"https://example.org/b.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
common==1.0.0 \\
    --hash=sha256:aaaa
    # via
    #   pkg_a
    #   pkg_b

pkg_a==0.1.0 \\
    --hash=sha256:bbbb

pkg_b==0.2.0 \\
    --hash=sha256:cccc
""")

_tests.append(_test_multiple_dependents)

def _test_git_source_skipped(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.1.0","source":{"git":"https://github.com/org/foo.git"}},
        {"name":"bar","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/bar.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
bar==0.0.1 \\
    --hash=sha256:deadbeef
""")

_tests.append(_test_git_source_skipped)

def _test_virtual_source_skipped(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"virtual-pkg","version":"0.0.0","source":{"virtual":true}},
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo==0.0.1 \\
    --hash=sha256:deadbeef
""")

_tests.append(_test_virtual_source_skipped)

def _test_sdist_hash(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"bar","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"sdist":{"hash":"sha256:deadb00f","url":"https://example.org/bar.tar.gz"}}
    ]}"""))
    env.expect.that_str(got).equals("""\
bar==0.0.1 \\
    --hash=sha256:deadb00f
""")

_tests.append(_test_sdist_hash)

def _test_wheel_and_sdist_hashes(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"sdist":{"hash":"sha256:feedcafe","url":"https://example.org/foo.tar.gz"},"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo==0.0.1 \\
    --hash=sha256:deadbeef \\
    --hash=sha256:feedcafe
""")

_tests.append(_test_wheel_and_sdist_hashes)

def _test_multiple_versions_same_package(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"resolution-markers":["python_full_version < '3.10'"],"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo-0.0.1.whl"}]},
        {"name":"foo","version":"0.0.2","source":{"registry":"https://pypi.org/simple"},"resolution-markers":["python_full_version >= '3.10'"],"wheels":[{"hash":"sha256:deadb11f","url":"https://example.org/foo-0.0.2.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo==0.0.1 ; python_full_version < '3.10' \\
    --hash=sha256:deadbeef

foo==0.0.2 ; python_full_version >= '3.10' \\
    --hash=sha256:deadb11f
""")

_tests.append(_test_multiple_versions_same_package)

def _test_dep_with_extras(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]},
        {"name":"bar","version":"0.0.2","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"foo","extra":["extra1"]}],"wheels":[{"hash":"sha256:baadbeef","url":"https://example.org/bar.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo[extra1]==0.0.1 \\
    --hash=sha256:deadbeef
    # via bar

bar==0.0.2 \\
    --hash=sha256:baadbeef
""")

_tests.append(_test_dep_with_extras)

def _test_multiple_hashes_from_wheels(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[
            {"hash":"sha256:aaaa","url":"https://example.org/foo-0.0.1-cp39.whl"},
            {"hash":"sha256:bbbb","url":"https://example.org/foo-0.0.1-py3-none-any.whl"}
        ]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo==0.0.1 \\
    --hash=sha256:aaaa \\
    --hash=sha256:bbbb
""")

_tests.append(_test_multiple_hashes_from_wheels)

def _test_package_no_hashes_no_deps(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"}}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo==0.0.1
""")

_tests.append(_test_package_no_hashes_no_deps)

def _test_package_with_multiple_optional_deps(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"bar","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:baadbeef","url":"https://example.org/bar.whl"}]},
        {"name":"baz","version":"0.0.2","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:deadc0de","url":"https://example.org/baz.whl"}]},
        {"name":"foo","version":"0.0.3","source":{"registry":"https://pypi.org/simple"},"optional-dependencies":{"extra1":[{"name":"bar"}],"extra2":[{"name":"baz"}]},"wheels":[{"hash":"sha256:feedcafe","url":"https://example.org/foo.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
bar==0.0.1 \\
    --hash=sha256:baadbeef
    # via foo

baz==0.0.2 \\
    --hash=sha256:deadc0de
    # via foo

foo[extra1,extra2]==0.0.3 \\
    --hash=sha256:feedcafe
""")

_tests.append(_test_package_with_multiple_optional_deps)

def _test_dep_with_multiple_extras(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"foo","version":"0.0.1","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:deadbeef","url":"https://example.org/foo.whl"}]},
        {"name":"bar","version":"0.0.2","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"foo","extra":["extra1","extra2"]}],"wheels":[{"hash":"sha256:baadbeef","url":"https://example.org/bar.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
foo[extra1,extra2]==0.0.1 \\
    --hash=sha256:deadbeef
    # via bar

bar==0.0.2 \\
    --hash=sha256:baadbeef
""")

_tests.append(_test_dep_with_multiple_extras)

def _test_extras_from_multiple_dependents(env):
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"common","version":"1.0.0","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:aaaa","url":"https://example.org/common.whl"}]},
        {"name":"pkg_a","version":"0.1.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"common","extra":["extra1"]}],"wheels":[{"hash":"sha256:bbbb","url":"https://example.org/a.whl"}]},
        {"name":"pkg_b","version":"0.2.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"common","extra":["extra2"]}],"wheels":[{"hash":"sha256:cccc","url":"https://example.org/b.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
common[extra1,extra2]==1.0.0 \\
    --hash=sha256:aaaa
    # via
    #   pkg_a
    #   pkg_b

pkg_a==0.1.0 \\
    --hash=sha256:bbbb

pkg_b==0.2.0 \\
    --hash=sha256:cccc
""")

_tests.append(_test_extras_from_multiple_dependents)

def _test_requires_dist_extras(env):
    """Test that extras from metadata.requires-dist are included."""
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"pytest-bazel","version":"0.1.6","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:a29e80e1d67c3db801bdd4d0b6b742f2bfb48cd6841caa33401458e5c4e29c21","url":"https://example.org/pytest_bazel-0.1.6.whl"}]},
        {"name":"root-pkg","version":"0.0.0","source":{"virtual":"."},"dependencies":[{"name":"pytest-bazel"}],"metadata":{"requires-dist":[{"name":"pytest-bazel","extras":["all"]}]}}
    ]}"""))
    env.expect.that_str(got).equals("""\
pytest-bazel[all]==0.1.6 \\
    --hash=sha256:a29e80e1d67c3db801bdd4d0b6b742f2bfb48cd6841caa33401458e5c4e29c21
    # via root-pkg
""")

_tests.append(_test_requires_dist_extras)

def _test_transitive_dependency_edge_markers(env):
    """Test that markers on dependency edges propagate transitively."""
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"cryptography","version":"50.0.2","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:cccc","url":"https://example.org/cryptography.whl"}]},
        {"name":"keyring","version":"25.7.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"secretstorage","marker":"sys_platform == 'linux'"}],"wheels":[{"hash":"sha256:aaaa","url":"https://example.org/keyring.whl"}]},
        {"name":"root-pkg","version":"0.0.0","source":{"virtual":"."},"dependencies":[{"name":"keyring","marker":"platform_machine != 's390x'"}]},
        {"name":"secretstorage","version":"3.5.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"cryptography"}],"wheels":[{"hash":"sha256:bbbb","url":"https://example.org/secretstorage.whl"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
cryptography==50.0.2 ; platform_machine != 's390x' and sys_platform == 'linux' \\
    --hash=sha256:cccc
    # via secretstorage

keyring==25.7.0 ; platform_machine != 's390x' \\
    --hash=sha256:aaaa
    # via root-pkg

secretstorage==3.5.0 ; platform_machine != 's390x' and sys_platform == 'linux' \\
    --hash=sha256:bbbb
    # via keyring
""")

_tests.append(_test_transitive_dependency_edge_markers)

def _test_diamond_dependency_edge_markers(env):
    """Test diamond paths with conditional vs unconditional edges."""
    got = uv_lock_to_requirements(json.decode("""{"package":[
        {"name":"both-conditional","version":"1.0.0","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:1111","url":"https://example.org/bc.whl"}]},
        {"name":"one-unconditional","version":"1.0.0","source":{"registry":"https://pypi.org/simple"},"wheels":[{"hash":"sha256:2222","url":"https://example.org/ou.whl"}]},
        {"name":"pkg-a","version":"0.1.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"both-conditional","marker":"sys_platform == 'linux'"},{"name":"one-unconditional","marker":"sys_platform == 'linux'"}],"wheels":[{"hash":"sha256:aaaa","url":"https://example.org/a.whl"}]},
        {"name":"pkg-b","version":"0.2.0","source":{"registry":"https://pypi.org/simple"},"dependencies":[{"name":"both-conditional","marker":"sys_platform == 'win32'"},{"name":"one-unconditional"}],"wheels":[{"hash":"sha256:bbbb","url":"https://example.org/b.whl"}]},
        {"name":"root-pkg","version":"0.0.0","source":{"virtual":"."},"dependencies":[{"name":"pkg-a"},{"name":"pkg-b"}]}
    ]}"""))
    env.expect.that_str(got).equals("""\
both-conditional==1.0.0 ; sys_platform == 'linux' or sys_platform == 'win32' \\
    --hash=sha256:1111
    # via
    #   pkg-a
    #   pkg-b

one-unconditional==1.0.0 \\
    --hash=sha256:2222
    # via
    #   pkg-a
    #   pkg-b

pkg-a==0.1.0 \\
    --hash=sha256:aaaa
    # via root-pkg

pkg-b==0.2.0 \\
    --hash=sha256:bbbb
    # via root-pkg
""")

_tests.append(_test_diamond_dependency_edge_markers)

def uv_lock_to_requirements_test_suite(name):
    test_suite(name = name, basic_tests = _tests)
