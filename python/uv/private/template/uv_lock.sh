#!/usr/bin/env bash
set -euxo pipefail

if [[ -n "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
    exec "{{args}}" "$@"
fi

# Build action mode: run uv, then copy the output from the project
# directory to the declared output file.
readonly out="{{out}}"
if [[ -f "{{src_out}}" ]]; then
    rm "{{src_out}}"
fi
"$@"
cp "{{src_out}}" "$out"
