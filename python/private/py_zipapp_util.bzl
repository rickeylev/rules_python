def _create_executable_zip_file(
        ctx,
        *,
        output,
        zip_file,
        stage2_bootstrap,
        runtime_details,
        venv):
    prelude = ctx.actions.declare_file(
        "{}_zip_prelude.sh".format(output.basename),
        sibling = output,
    )
    if stage2_bootstrap:
        _create_stage1_bootstrap(
            ctx,
            output = prelude,
            stage2_bootstrap = stage2_bootstrap,
            runtime_details = runtime_details,
            is_for_zip = True,
            venv = venv,
        )
    else:
        ctx.actions.write(prelude, "#!/usr/bin/env python3\n")

    ctx.actions.run_shell(
        command = "cat {prelude} {zip} > {output}".format(
            prelude = prelude.path,
            zip = zip_file.path,
            output = output.path,
        ),
        inputs = [prelude, zip_file],
        outputs = [output],
        use_default_shell_env = True,
        mnemonic = "PyBuildExecutableZip",
        progress_message = "Build Python zip executable: %{label}",
    )
