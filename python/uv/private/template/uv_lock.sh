#!/usr/bin/env bash
set -euxo pipefail

if [[ -n "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
    readonly out="${BUILD_WORKSPACE_DIRECTORY}/{{src_out}}"
else
    # uv normally works with files in the project directory, so we have to do
    # some careful shuffling around to get it to work. Our strategy is to:
    #   1. Remove the existing output from the file, which is a read-only
    #      symlink to the src tree.
    #   2. Do a symlink to the output directory and replace the existing
    #      output.
    #
    # This ensures that whatever we do the uv.lock is generated into the output
    # directory in the sandbox.
    readonly out="{{out}}"
    if [[ -f "{{src_out}}" ]]; then
        cp "{{src_out}}" "$out"
        rm "{{src_out}}"
    fi
    ln -s "$(pwd)"/"$out" "{{src_out}}"
fi
exec "{{args}}" "$@"
