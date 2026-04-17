load("//python/private:attributes.bzl", "CONFIG_SETTINGS_ATTR", "apply_config_settings_attr")
load("//python/private:rule_builders.bzl", "ruleb")
load("//python/private:builders.bzl", "builders")
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

def _maybe_add_provider(providers, seen_providers, target, provider_type):
    if provider_type in target:
        if provider_type in seen_providers:
            fail("Provider {} provided by both {} and {}".format(
                provider_type, seen_providers[provider_type], target.label))
        seen_providers[provider_type] = target.label
        providers.append(target[provider_type])

def _py_transition_impl(ctx):
    files = builders.DepsetBuilder()
    runfiles = builders.RunfilesBuilder()
    providers = []
    
    seen_providers = {}
    
    for target in ctx.attr.targets:
        files.add(target[DefaultInfo].files)
        runfiles.add(target[DefaultInfo].default_runfiles)
        runfiles.add(target[DefaultInfo].data_runfiles)
        runfiles.add(ctx.runfiles(transitive_files = target[DefaultInfo].files))
        
        _maybe_add_provider(providers, seen_providers, target, PyRuntimeInfo)
            
    default_info = DefaultInfo(
        files = files.build(),
        runfiles = runfiles.build(ctx)
    )
    providers.append(default_info)
    return providers

py_transition = ruleb.Rule(
    implementation = _py_transition_impl,
    attrs = {
        "targets": attr.label_list(cfg = _py_transition_transition),
        "_allowlist_function_transition": attr.label(
            default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
        ),
    } | CONFIG_SETTINGS_ATTR,
).build()
