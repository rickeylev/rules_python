# Extending the rules

:::{important}
**This is public, but unstable, functionality.**

Extending and customizing the rules is supported functionality, but with weaker
backwards compatibility guarantees, and is not fully subject to the normal
backwards compatibility procedures and policies. It's simply not feasible to
support every possible customization with strong backwards compatibility
guarantees.
:::

Because of the rich ecosystem of tools and variety of use cases, APIs are
provided to make it easy to create custom rules using the existing rules as a
basis. This allows implementing behaviors that aren't possible using
wrapper macros around the core rules, and can make certain types of changes
much easier and transparent to implement.

:::{note}
It is not required to extend a core rule. The minimum requirement for a custom
rule is to return the {bzl:obj}`PyInfo` provider. Extending the core rules is
most useful when you want all or most of the behavior of a core rule.
:::

## Creating custom rules

Custom rules can be created using the core rules as a basis by using their rule
builder APIs.

* {bzl:obj}`//python/apis:executables.bzl%executables` for builders for executables
* {bzl:obj}`//python/apis:libraries.bzl%libraries` for builders for libraries

These builders create {obj}`attrb.Rule` objects, which are thin
wrappers around the keyword arguments eventually passed to the `rule()`
function. These builder APIs give access the the _entire_ rule definition and
allow arbitrary modifications.

### Example: validating a source file

In this example, we create a `py_library` replacement that verifies source
code contains the word "snakes". It does this by:

* Adding an implicit dependency on a checker program
* Calling the base implementation function
* Running the checker on the srcs files
* Adding the result to the `_validation` output group (a special output
  group for validation behaviors).

To users, they can use `has_snakes_library` the same as `py_library`. The same
is true for other targets that might consume the rule.

```
load("@rules_python//python/api:libraries.bzl", "libraries")
load("@rules_python//python/api:attr_builders.bzl", "attrb")

def _has_snakes_impl(ctx, base):
    providers = base(ctx)

    out = ctx.actions.declare_file(ctx.label.name + "_snakes.check")
    ctx.actions.run(
        inputs = ctx.files.srcs,
        outputs = [out],
        executable = ctx.attr._checker[DefaultInfo].files_to_run,
        args = [out.path] + [f.path for f in ctx.files.srcs],
    )
    prior_ogi = None
    for i, p in enumerate(providers):
        if type(p) == "OutputGroupInfo":
            prior_ogi = (i, p)
            break
    if prior_ogi:
        groups = {k: getattr(prior_ogi[1], k) for k in dir(prior_ogi)}
        if "_validation" in groups:
            groups["_validation"] = depset([out], transitive=groups["_validation"])
        else:
            groups["_validation"] = depset([out])
        providers[prior_ogi[0]] = OutputGroupInfo(**groups)
    else:
        providers.append(OutputGroupInfo(_validation=depset([out])))
    return providers

def create_has_snakes_rule():
    r = libraries.py_library_builder()
    base_impl = r.implementation()
    r.set_implementation(lambda ctx: _has_snakes_impl(ctx, base_impl))
    r.attrs["_checker"] = attrb.Label(
        default="//:checker",
        executable = True,
    )
    return r.build()
has_snakes_library = create_has_snakes_rule()
```
