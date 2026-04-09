def _rel_symlink_impl(ctx):
    symlink = ctx.actions.declare_symlink(ctx.label.name)
    ctx.actions.symlink(
        output = symlink,
        target_path = ctx.attr.target_path,
    )
    return [DefaultInfo(files = depset([symlink]), runfiles = ctx.runfiles(files = [symlink]))]

rel_symlink = rule(
    implementation = _rel_symlink_impl,
    attrs = {
        "target_path": attr.string(mandatory = True),
    },
)
