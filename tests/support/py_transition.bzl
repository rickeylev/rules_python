load("//python/private:attributes.bzl", "CONFIG_SETTINGS_ATTR", "apply_config_settings_attr")
load("//python/private:attr_builders.bzl", "attrb")
load("//python/private:transition_labels.bzl", "TRANSITION_LABELS")
load("//python:py_runtime_info.bzl", "PyRuntimeInfo")

def _transition_impl(settings, attr):
    new_settings = dict(settings)
    return apply_config_settings_attr(new_settings, attr)

_py_transition_transition = transition(
    implementation = _transition_impl,
    inputs = TRANSITION_LABELS,
    outputs = TRANSITION_LABELS,
)

def _py_transition_impl(ctx):
    all_files = []
    all_runfiles = []
    providers = []
    
    for target in ctx.attr.targets:
        all_files.append(target[DefaultInfo].files)
        all_runfiles.append(target[DefaultInfo].default_runfiles)
        all_runfiles.append(target[DefaultInfo].data_runfiles)
        all_runfiles.append(ctx.runfiles(transitive_files = target[DefaultInfo].files))
        
    default_info = DefaultInfo(
        files = depset(transitive = all_files),
        runfiles = ctx.runfiles().merge_all(all_runfiles)
    )
    providers.append(default_info)
    
    if len(ctx.attr.targets) == 1:
        target = ctx.attr.targets[0]
        if PyRuntimeInfo in target:
            providers.append(target[PyRuntimeInfo])
            
    return providers

attrs = {
    "targets": attr.label_list(cfg = _py_transition_transition),
    "_allowlist_function_transition": attr.label(
        default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
    ),
    "config_settings": CONFIG_SETTINGS_ATTR["config_settings"]().build(),
}

py_transition = rule(
    implementation = _py_transition_impl,
    attrs = attrs,
)
