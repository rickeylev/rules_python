#!/usr/bin/env bash

set -euo pipefail

zipapp="$1"
output="$("$zipapp")"
expected="Hello from zipapp"

if [[ "$output" != "$expected" ]]; then
  echo "Expected output '$expected', but got '$output'"
  exit 1
fi
