"""Rule for zipping the Python standard library."""

def _zip_stdlib_impl(ctx):
    output = ctx.outputs.zip
    srcs = ctx.files.srcs

    manifest_content = []
    for f in srcs:
        # The short_path for files from an external repo will be
        # ../<repo_name>/path/to/file. We need to find the `lib` dir.
        parts = f.short_path.split("/")
        lib_index = -1
        for i, part in enumerate(parts):
            if part == "lib":
                lib_index = i
                break

        if lib_index == -1 or lib_index + 1 >= len(parts) or not parts[lib_index + 1].startswith("python"):
            fail("Source file '{}' is not in a recognizable stdlib directory structure (lib/pythonX.Y*/...)".format(f.short_path))

        dest_path = "/".join(parts[lib_index + 2:])
        manifest_content.append("{}={}".format(dest_path, f.path))

    manifest = ctx.actions.declare_file(ctx.attr.name + "_manifest.txt")
    ctx.actions.write(manifest, "\n".join(manifest_content))

    args = ctx.actions.args()
    args.add("cC")
    args.add(output.path)
    args.add("@" + manifest.path)

    ctx.actions.run(
        executable = ctx.executable._zipper,
        arguments = [args],
        inputs = depset(srcs, transitive = [depset([manifest])]),
        outputs = [output],
        mnemonic = "PyZipStdlib",
        progress_message = "Zipping stdlib for {}".format(ctx.label),
    )

zip_stdlib = rule(
    implementation = _zip_stdlib_impl,
    attrs = {
        "srcs": attr.label_list(allow_files = True),
        "zip": attr.output(mandatory = True),
        "_zipper": attr.label(
            cfg = "exec",
            executable = True,
            default = "@bazel_tools//tools/zip:zipper",
        ),
    },
)
