# todo: use camelCase for var names. It seems to be the PS idiom
# Equivalent to set -e
$ErrorActionPreference = 'Stop'

# todo: remove this
$Env:RULES_PYTHON_BOOTSTRAP_VERBOSE = "1"

# Check if verbose mode is requested (using environment variable)
if ($Env:RULES_PYTHON_BOOTSTRAP_VERBOSE -eq "1") {
    # NOTE: Set-PSDebug affects the calling shell.
    # Set-PSDebug -Trace 2
    $VerbosePreference = 'Continue' # Make Write-Verbose print and continue
}

# runfiles-relative path to stage2 bootstrap file
$STAGE2_BOOTSTRAP = "%stage2_bootstrap%"
# runfiles-relative path to python interpreter to use
$PYTHON_BINARY = '%python_binary%'
# The path that PYTHON_BINARY should symlink to.
# runfiles-relative path, absolute path, or single word.
# Only applicable for zip files or when venv is recreated at runtime.
$PYTHON_BINARY_ACTUAL = "%python_binary_actual%"

# 0 or 1 (Treating as strings as in the original script, PowerShell would typically use $true/$false)
$IS_ZIPFILE = "%is_zipfile%"
# 0 or 1
$RECREATE_VENV_AT_RUNTIME = "%recreate_venv_at_runtime%"

# array of strings
# Placeholders need to be PowerShell array syntax compatible when replaced, e.g., 'arg1', 'arg2'
# todo: parse from string
$INTERPRETER_ARGS_FROM_TARGET = @(
    #"%interpreter_args%"
)

Write-Verbose "STAGE2_BOOTSTRAP: $STAGE2_BOOTSTRAP"
Write-Verbose "PYTHON_BINARY: $PYTHON_BINARY"
Write-Verbose "PYTHON_BINARY_ACTUAL: $PYTHON_BINARY_ACTUAL"
Write-Verbose "IS_ZIPFILE: $IS_ZIPFILE"
Write-Verbose "RECREATE_VENV_AT_RUNTIME: $RECREATE_VENV_AT_RUNTIME"

# --- Main Logic ---

try { # Wrap main logic for potential cleanup using finally (mimics trap EXIT)
    Write-Verbose "Processing as non-Zipfile..."
    # Function to find Runfiles Root (PowerShell adaptation)
    Write-Verbose "Locating runfiles based on:"
    Write-Verbose "  RUNFILES_DIR=${Env:RUNFILES_PATH}"
    Write-Verbose "  argv0=$PSCommandPath"
    if ($Env:RUNFILES_DIR -ne $null) {
        Write-Verbose "Runfiles found via RUNFILES_DIR envvar"
        $RUNFILES_DIR = $Env:RUNFILES_DIR
    } elseif (Test-Path -Path "${PSCommandPath}.runfiles") {
        Write-Verbose "Runfiles found via arg0 sibling .runfiles directory"
        $RUNFILES_DIR = "${PSCommandPath}.runfiles"
    } else {
        # Otherwise, we are probably a program within another program's
        # runfiles tree. Search up to find the runfiles directory.
        $currentDir = Split-Path -Path "$PSCommandPath" -Parent
        while ($currentDir -ne "") {
            if ($currentDir -like "*.runfiles") {
                $RUNFILES_DIR = $currentDir
                Write-Verbose "Runfiles found via parent directory search"
                break
            }
            $currentDir = Split-Path -Path "$currentDir" -Parent
        }
    }
    if ($RUNFILES_DIR -eq $null) {
        throw "ERROR: Unable to find runfiles directory for $PSCommandPath"
    }

    $runfilesVenvRoot = Split-Path -Parent (Split-Path -Parent $PYTHON_BINARY)

    # todo: also check RULES_PYTHON_EXTRACT_ROOT
    $tempDirRoot = $Env:TEMP
    $tempDirName = "rp-venv-" + (New-Guid).ToString()
    #$venvRoot = Join-Path -Path $tempDirRoot -ChildPath $tempDirName
    # todo: Remove this debug path
    $venvRoot = "C:\tempvenv"
    Write-Verbose "Temp venv root: $venvRoot"
    Remove-Item -Path $venvRoot -Recurse -Force
    New-Item -ItemType Directory -Path $venvRoot | Out-Null
    New-Item -ItemType Directory -Path $venvRoot\Scripts | Out-Null
    $pyBasename = Split-Path $PYTHON_BINARY_ACTUAL -Leaf
    $pyExe = "$venvRoot\Scripts\$pyBasename"
    New-Item -ItemType SymbolicLink -Path "$pyExe" `
        -Target "$RUNFILES_DIR\$PYTHON_BINARY_ACTUAL" | Out-Null
    $pyActualDir = Split-Path -Parent "$RUNFILES_DIR\$PYTHON_BINARY_ACTUAL"
    Write-Verbose "py actual dir: $pyActualDir"
    # todo: have build system pass these paths along
    New-Item -ItemType SymbolicLink -Path $venvRoot\Scripts\python3.dll `
        -Target "$pyActualDir\python3.dll"
    New-Item -ItemType SymbolicLink -Path $venvRoot\Scripts\python311.dll `
        -Target "$pyActualDir\python311.dll"

    New-Item -ItemType SymbolicLink -Path $venvRoot\Lib `
        -Target $runfilesVenvRoot\Lib | Out-Null
    # todo: Why must we set home again? ISTR that windows python doesn't
    # handle a missing home key as nicely as linux, but don't remember
    # the details
    # todo: call python to get this value; see stage1.sh and its logic
    ##New-Item -ItemType File -Path "$venvRoot\pyvenv.cfg"
    "home = $pyActualDir" >> "$venvRoot\pyvenv.cfg"


    # --- Interpreter Check ---
    # PowerShell doesn't have a '-x' check like Unix, so just check existence.
    if (-not (Test-Path $pyExe)) {
        throw "ERROR: Python interpreter not found: $pyExe"
    }

    $stage2Path = Join-Path $RUNFILES_DIR $STAGE2_BOOTSTRAP

    # --- Prepare Arguments and Environment ---
    $pyEnv = @{} # Use a hashtable for environment variables
    $interpreter_args_list = [System.Collections.Generic.List[string]]::new()

    # Default to PYTHONSAFEPATH=1, but respect the outer value if it's set.
    if ($Env:PYTHONSAFEPATH -eq $null) {
        # If PYTHONSAFEPATH is not set at all in the environment, default it to 1
        $pyEnv['PYTHONSAFEPATH'] = '1'
    } else {
        # If it is set (even to empty string), pass its value through
        $pyEnv['PYTHONSAFEPATH'] = $Env:PYTHONSAFEPATH
    }


    # todo: handle zip files
    if ($IS_ZIPFILE -eq "1") {
        $interpreter_args_list.Add("-XRULES_PYTHON_ZIP_DIR=$zip_dir")
        Write-Verbose "Adding zip dir arg: -XRULES_PYTHON_ZIP_DIR=$zip_dir"
    }

    if (-not [string]::IsNullOrEmpty($Env:RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS)) {
        # Split the string into an array of args (basic space splitting)
        $additional_interpreter_args = $Env:RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS -split ' '
        foreach ($arg in $additional_interpreter_args) {
            $interpreter_args_list.Add($arg)
        }
        Write-Verbose "Adding additional interpreter args: $($additional_interpreter_args -join ' ')"
        # Remove-Item Env:\RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS # Equivalent to unset (optional)
    }

    # Set after custom user env vars
    $pyEnv["RUNFILES_DIR"] = $RUNFILES_DIR

    # Combine all arguments for the python process
    $argv = @()
    $argv += $interpreter_args_list # Add args derived in this script
    $argv += $INTERPRETER_ARGS_FROM_TARGET # Add args from build target
    $argv += $stage2Path # Add the stage2 script
    $argv += $args # Add arguments passed to this bootstrap script ($@)

    $Env:RULES_PYTHON_BOOTSTRAP_VERBOSE = ""
    # IMPORTANT: Modifications to $Env affect the caller because powershell
    # scripts are run *in-process*.
    $origEnv = @{}
    foreach ($key in $pyEnv.Keys) {
        if (Test-Path Env:$key) {
            $origEnv[$key] = Get-Item "Env:$key"
        } else {
            $origEnv[$key] = $null
        }
    }
    try {
        Write-Verbose "Executing:"
        foreach($key in $pyEnv.Keys) {
            $value = $pyEnv[$key]
            Set-Item "Env:$key" $value
            Write-Verbose "  $key=$value"
        }
        Write-Verbose "  Python: $pyExe"
        Write-Verbose "  Arguments:"
        for ($i=0; $i -lt $argv.Length; $i++) {
            $value = $argv[$i]
            Write-Verbose "  [$i] $value"
        }
        # NOTE: We use & (call operator) instead of Start-Process because
        # it's more appropirate for running a synchronous console program.
        # It connects the standards streams, perserves the exit code, etc.
        & "$pyExe" $argv
    } finally {
        foreach ($key in $origEnv.Keys) {
            Set-Item "Env:$key" $origEnv[$key]
        }
    }

    # Capture the exit code of the last command
    $lastExitCode = $LASTEXITCODE
    Write-Verbose "Python process exit code: $lastExitCode"
    exit $lastExitCode
} finally {
  Write-Verbose "Deleting temp venv: $venvRoot"
  #Remove-Item -Path $venvRoot -Recurse -Force
}

# The script should exit within the try block or via error preference.
# This is just a fallback.
exit 0
