#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
    readonly out="${BUILD_WORKSPACE_DIRECTORY}/{{src_out}}"
else
    readonly out="{{out}}"
    cp "{{src_out}}" "$out"
fi
exec "{{args}}" --output-file "$out" "$@"
