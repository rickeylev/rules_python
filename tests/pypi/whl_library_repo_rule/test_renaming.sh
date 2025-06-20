#!/bin/bash
set -euo pipefail

# Args:
# 1. Path to pkg_with_build_files/_BUILD
# 2. Path to pkg_with_build_files/__BUILD (original BUILD)
# 3. Path to pkg_with_build_files/_BUILD.bazel
# 4. Path to data_files/_BUILD
# 5. Negative check: original pkg_with_build_files/BUILD (relative path string)
# 6. Negative check: original pkg_with_build_files/BUILD.bazel (relative path string)
# 7. Negative check: original data_files/BUILD (relative path string)
# 8. Path to __init__.py (to check other files are present)
# 9. Path to some_data.txt
# 10. Path to another_data.txt
# 11. Path to conflict_data.txt
# 12. Path to data_specific_file.txt

EXPECTED_PKG_UNDERSCORE_BUILD="$1"
EXPECTED_PKG_DOUBLE_UNDERSCORE_BUILD="$2"
EXPECTED_PKG_UNDERSCORE_BUILD_BAZEL="$3"
EXPECTED_DATA_UNDERSCORE_BUILD="$4"

ORIGINAL_PKG_BUILD_REL_PATH="$5"
ORIGINAL_PKG_BUILD_BAZEL_REL_PATH="$6"
ORIGINAL_DATA_BUILD_REL_PATH="$7"

PKG_INIT_PY="$8"
PKG_SOME_DATA_TXT="$9"
PKG_ANOTHER_DATA_TXT="${10}"
PKG_CONFLICT_DATA_TXT="${11}"
DATA_SPECIFIC_FILE_TXT="${12}"


echo "--- Checking existence of renamed files ---"
[ -f "$EXPECTED_PKG_UNDERSCORE_BUILD" ] || (echo "Expected $EXPECTED_PKG_UNDERSCORE_BUILD to exist" && exit 1)
echo "$EXPECTED_PKG_UNDERSCORE_BUILD exists."

[ -f "$EXPECTED_PKG_DOUBLE_UNDERSCORE_BUILD" ] || (echo "Expected $EXPECTED_PKG_DOUBLE_UNDERSCORE_BUILD to exist (original BUILD after conflict)" && exit 1)
echo "$EXPECTED_PKG_DOUBLE_UNDERSCORE_BUILD exists."

[ -f "$EXPECTED_PKG_UNDERSCORE_BUILD_BAZEL" ] || (echo "Expected $EXPECTED_PKG_UNDERSCORE_BUILD_BAZEL to exist" && exit 1)
echo "$EXPECTED_PKG_UNDERSCORE_BUILD_BAZEL exists."

[ -f "$EXPECTED_DATA_UNDERSCORE_BUILD" ] || (echo "Expected $EXPECTED_DATA_UNDERSCORE_BUILD to exist" && exit 1)
echo "$EXPECTED_DATA_UNDERSCORE_BUILD exists."

echo "--- Checking non-existence of original BUILD files ---"
# These paths are relative to the bazel-testlogs directory (or where the test runs)
# and point into the external repo if correctly constructed.
# The path needs to be ../<repo_name>/<path_inside_repo>
# The arguments ORIGINAL_PKG_BUILD_REL_PATH are already in the form @repo//path/to/file
# We need to convert @repo//path to ../repo/path for file system checks.

# Helper function to convert label-like path to filesystem path
label_to_fs_path() {
    local label_path=$1
    # Remove @ at the beginning
    local no_at=${label_path#@}
    # Replace // with /
    local fs_path="../${no_at/\/\///}"
    echo "$fs_path"
}

ORIGINAL_PKG_BUILD_FS_PATH=$(label_to_fs_path "$ORIGINAL_PKG_BUILD_REL_PATH")
ORIGINAL_PKG_BUILD_BAZEL_FS_PATH=$(label_to_fs_path "$ORIGINAL_PKG_BUILD_BAZEL_REL_PATH")
ORIGINAL_DATA_BUILD_FS_PATH=$(label_to_fs_path "$ORIGINAL_DATA_BUILD_REL_PATH")

[ ! -f "$ORIGINAL_PKG_BUILD_FS_PATH" ] || (echo "Expected $ORIGINAL_PKG_BUILD_FS_PATH to NOT exist" && exit 1)
echo "$ORIGINAL_PKG_BUILD_FS_PATH does not exist."

[ ! -f "$ORIGINAL_PKG_BUILD_BAZEL_FS_PATH" ] || (echo "Expected $ORIGINAL_PKG_BUILD_BAZEL_FS_PATH to NOT exist" && exit 1)
echo "$ORIGINAL_PKG_BUILD_BAZEL_FS_PATH does not exist."

[ ! -f "$ORIGINAL_DATA_BUILD_FS_PATH" ] || (echo "Expected $ORIGINAL_DATA_BUILD_FS_PATH to NOT exist" && exit 1)
echo "$ORIGINAL_DATA_BUILD_FS_PATH does not exist."


echo "--- Checking existence of other files from the wheel ---"
[ -f "$PKG_INIT_PY" ] || (echo "Expected $PKG_INIT_PY to exist" && exit 1)
echo "$PKG_INIT_PY exists."
[ -f "$PKG_SOME_DATA_TXT" ] || (echo "Expected $PKG_SOME_DATA_TXT to exist" && exit 1)
echo "$PKG_SOME_DATA_TXT exists."
[ -f "$PKG_ANOTHER_DATA_TXT" ] || (echo "Expected $PKG_ANOTHER_DATA_TXT to exist" && exit 1)
echo "$PKG_ANOTHER_DATA_TXT exists."
[ -f "$PKG_CONFLICT_DATA_TXT" ] || (echo "Expected $PKG_CONFLICT_DATA_TXT to exist" && exit 1)
echo "$PKG_CONFLICT_DATA_TXT exists."
[ -f "$DATA_SPECIFIC_FILE_TXT" ] || (echo "Expected $DATA_SPECIFIC_FILE_TXT to exist" && exit 1)
echo "$DATA_SPECIFIC_FILE_TXT exists."


echo "--- All checks passed ---"
exit 0
