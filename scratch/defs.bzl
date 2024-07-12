def gen_autoconf(**kwargs):
    _gen_autoconf(
        **kwargs
    )

def _gen_autoconf_impl(ctx):
    output = ctx.actions.declare_file(ctx.label.name)

    vars = {}
    for dep in ctx.attr.deps:
        vars.update(dep[AutoConfInfo].vars.to_list())

    subs = {}
    for var_name, value in vars.items():
        sub_name = "#undef {}".format(var_name)
        if value == "comment-out":
            sub_value = "/* {} */".format(sub_name)
        else:
            sub_value = "#define {} {}".format(var_name, value)
        subs[sub_name] = sub_value

    ctx.actions.expand_template(
        template = ctx.file.src,
        output = output,
        substitutions = subs,
    )
    return [DefaultInfo(
        files = depset([output]),
    )]

_gen_autoconf = rule(
    implementation = _gen_autoconf_impl,
    attrs = {
        "src": attr.label(allow_single_file = True),
        "deps": attr.label_list(),
    },
)

AutoConfInfo = provider()

def _autoconf_vars_impl(ctx):
    vars = depset(
        direct = [tuple(v) for v in ctx.attr.vars.items()],
        transitive = [dep[AutoConfInfo].vars for dep in ctx.attr.deps],
    )
    return [AutoConfInfo(
        vars = vars,
    )]

autoconf_vars = rule(
    implementation = _autoconf_vars_impl,
    attrs = {
        "vars": attr.string_dict(),
        "deps": attr.label_list(),
    },
)

autoconf = struct(
    gen_autoconf = gen_autoconf,
    autoconf_vars = autoconf_vars,
)
