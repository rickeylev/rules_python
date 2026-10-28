"""A WORKSPACE repository rule for installing PyPI dependencies from uv.lock."""

load("@toml.bzl", "toml")
load("//python/private:repo_utils.bzl", "REPO_DEBUG_ENV_VAR")
load(":attrs.bzl", COMMON_ATTRS = "ATTRS")
load(":pip_repository.bzl", "render_pip_repository")
load(":pip_repository_attrs.bzl", "ATTRS")
load(":uv_lock_to_requirements.bzl", "uv_lock_to_requirements")

def _uv_lock_repo_impl(rctx):
    uv_lock_content = toml.decode(rctx.read(rctx.attr.uv_lock))
    requirements_txt = uv_lock_to_requirements(uv_lock_content)
    render_pip_repository(
        rctx,
        requirements_txt = requirements_txt,
    )

uv_lock_repo = repository_rule(
    attrs = dict(
        annotations = attr.string_dict(
            doc = """
Optional annotations to apply to packages. Keys should be package names, with
capitalization matching the input lockfile, and values should be generated
using the `package_annotation` macro.
""",
        ),
        experimental_requirement_cycles = ATTRS["experimental_requirement_cycles"],
        extra_hub_aliases = ATTRS["extra_hub_aliases"],
        use_hub_alias_dependencies = ATTRS["use_hub_alias_dependencies"],
        uv_lock = attr.label(
            allow_single_file = True,
            mandatory = True,
            doc = """
A resolved `uv.lock` file containing the transitive set of dependencies.
""",
        ),
        _config_template = attr.label(
            default = ":config.bzl.tmpl",
        ),
        _template = attr.label(
            default = ":requirements.bzl.tmpl.workspace",
        ),
        **COMMON_ATTRS
    ),
    doc = """
Accepts a resolved `uv.lock` file and installs the dependencies listed within.
""",
    implementation = _uv_lock_repo_impl,
    environ = [
        "RULES_PYTHON_PIP_ISOLATED",
        REPO_DEBUG_ENV_VAR,
    ],
)
