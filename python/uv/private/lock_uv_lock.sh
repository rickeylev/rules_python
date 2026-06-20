#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
    readonly out="${BUILD_WORKSPACE_DIRECTORY}/{{src_out}}"
else
    exit 1
fi

if [[ -f "$out" ]]; then
    # Ensure that the lock file is present in the output dir as a symlink - if it gets
    # changed, then the src also get changed. Remove anything that was added there by
    # bazel.
    rm "{{src_out}}"
    ln -s "$out" "{{src_out}}"
fi
exec "{{args}}" "$@"
