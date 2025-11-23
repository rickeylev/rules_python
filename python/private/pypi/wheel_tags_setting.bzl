load("//python/private:toolchain_types.bzl", "TARGET_TOOLCHAIN_TYPE")

_WHEEL_TAGS_TRUE = "TRUE"
_WHEEL_TAGS_FALSE = "FALSE"

def wheel_tags_setting(*, name, **wheel_tags):
    """Creates an env_marker setting.

    Generated targets:

    * `is_{name}_true`: config_setting that matches when the expression is true.
    * `{name}`: env marker target that evalutes the expression.

    Args:
        name: {type}`str` target name
        expression: {type}`str` the environment marker string to evaluate
        **kwargs: {type}`dict` additional common kwargs.
    """
    native.config_setting(
        name = "is_{}_true".format(name),
        flag_values = {
            ":{}".format(name): _WHEEL_TAGS_TRUE,
        },
    )
    _wheel_tags_setting(
        name = name,
        **wheel_tags
    )

def _wheel_tags_setting_impl(ctx):
    runtime = ctx.toolchains[TARGET_TOOLCHAIN_TYPE].py3_runtime

    pyimpl_is_compatible = False
    abi_is_compatible = False
    platform_is_compatible = False

    python_tag = ctx.attr.python_tag
    if python_tag in ("py3", "any"):
        pyimpl_is_compatible = True
    else:
        pass

    abi_tag = ctx.attr.abi_tag
    if abi_tag == "none":
        abi_is_compatible = True
    else:
        pass

    platform_tag = ctx.attr.platform_tag
    if platform_tag == "any":
        platform_is_compatible = True
    else:
        pass

    compatible = (pyimpl_is_compatible and
                  abi_is_compatible and
                  platform_is_compatible)
    value = "TRUE" if compatible else "FALSE"
    return [config_common.FeatureFlagInfo(value = value)]

_wheel_tags_setting = rule(
    implementation = _wheel_tags_setting_impl,
    attrs = {
        "python_tag": attr.string(),
        "abi_tag": attr.string(),
        "platform_tag": attr.string(),
    },
    toolchains = [
        config_common.toolchain_type(
            TARGET_TOOLCHAIN_TYPE,
            mandatory = False,
        ),
    ],
)
