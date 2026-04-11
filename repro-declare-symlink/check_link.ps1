echo "Hello and welcome to the"
echo "reproduction script for symlink"
echo "checking in bazel"
echo "On windows, that is, with declare_symlink"
echo "..."
echo "because it doesnt preserve the value correct"
echo "..."
echo "btw, windows is picky about relative symlinks"
echo "it requires uses backslashes, not forward slashes"
echo ""

$linkfile = "$Env:RUNFILES_DIR\_main\repro-declare-symlink\my_symlink"

Get-Item "$linkfile"
$actual = (Get-Item "$linkfile").Target

$expected = "..\subrepo\data.txt"

if ("$actual" -ne "$expected") {
    echo "expected: $expected"
    echo "  actual: $actual"
    exit 1
}
