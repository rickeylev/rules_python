# Equivalent to set -e
$ErrorActionPreference = 'Stop'

# Copied to local var for easier dev of the bootstrap
$bootstrapVerbose = $Env:RULES_PYTHON_BOOTSTRAP_VERBOSE
$bootstrapVerbose = "1" # todo: remove
# Check if verbose mode is requested (using environment variable)
if ($bootstrapVerbose -eq "1") {
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

# 0 or 1
$IS_ZIPFILE = "%is_zipfile%"
# 0 or 1
$RECREATE_VENV_AT_RUNTIME = "%recreate_venv_at_runtime%"
# 0 or 1
# If 1, then the path to python will be resolved by running
# PYTHON_BINARY_ACTUAL to determine the actual underlying interpreter.
# todo: implement this
$RESOLVE_PYTHON_BINARY_AT_RUNTIME="%resolve_python_binary_at_runtime%"

# array of strings
# The placeholder is expanded to quoted strings separated by newlines.
$INTERPRETER_ARGS_FROM_TARGET = @(
%interpreter_args%
)

Write-Verbose "Template expanded variables:"
Write-Verbose "  STAGE2_BOOTSTRAP: $STAGE2_BOOTSTRAP"
Write-Verbose "  PYTHON_BINARY: $PYTHON_BINARY"
Write-Verbose "  PYTHON_BINARY_ACTUAL: $PYTHON_BINARY_ACTUAL"
Write-Verbose "  IS_ZIPFILE: $IS_ZIPFILE"
Write-Verbose "  RECREATE_VENV_AT_RUNTIME: $RECREATE_VENV_AT_RUNTIME"
Write-Verbose "  RESOLVE_PYTHON_BINARY_AT_RUNTIME=$RESOLVE_PYTHON_BINARY_AT_RUNTIME"
Write-Verbose "  INTERPRETER_ARGS_FROM_TARGET: $INTERPRETER_ARGS_FROM_TARGET"

Write-Verbose "Notable environment variables:"
Write-Verbose "  RUNFILES_DIR=${Env:RUNFILES_DIR}"
Write-Verbose "  RUNFILES_MANIFEST_FILE=${Env:RUNFILES_MANIFEST_FILE}"

function newSymlink {
    param(
        [string]$Path,
        [string]$Target
    )
    Write-Verbose "Symlink: $Path -> $Target"
    New-Item -ItemType SymbolicLink -Path $Path -Target $Target | Out-Null
}


try {
    if ($RECREATE_VENV_AT_RUNTIME -eq "0") {
        # This shouldn't happen in practice -- it indicates a toolchain
        # misconfiguration. Ignore for now.
        Write-Warning "Unsupported: RECREATE_VENV_AT_RUNTIME=0: ignoring"
    }

    if ($IS_ZIPFILE -eq "1") {
        $zipDir = logic to create in temp or use extract root
        Extract-Archive -Path $PSCommandPath -DestinationPath $zipDir
        set runfilesdir to zip location
        try refactoring logic into clearn try/finaly block to manage
        extraction life cycle
    }
    Write-Verbose "Processing as non-Zipfile..."
    # Function to find Runfiles Root (PowerShell adaptation)
    Write-Verbose "Locating runfiles for: $PSCommandPath"
    $runfilesDir = $null
    if ($Env:RUNFILES_DIR -ne $null) {
        Write-Verbose "Runfiles found via RUNFILES_DIR envvar"
        $runfilesDir = $Env:RUNFILES_DIR
    } elseif ($Env:RUNFILES_MANIFEST_FILE -like "*.runfiles_manifest") {
        Write-Verbose "Runfiles found via RUNFILES_MANIFEST_FILE .runfiles_manifest"
        $runfilesDir = $Env:RUNFILES_MANIFEST_FILE -replace "[.]runfiles_manifest$", ".runfiles"
    } elseif ($Env:RUNFILES_MANIFEST_FILE -like "*.runfiles/MANIFEST") {
        Write-Verbose "Runfiles found via RUNFILES_MANIFEST_FILE /MANIFEST"
        $runfilesDir = $Env:RUNFILES_MANIFEST_FILE -replace "[.]runfiles/MANIFEST$", ".runfiles"
    } elseif (Test-Path -Path "${PSCommandPath}.runfiles") {
        Write-Verbose "Runfiles found via argv0 sibling .runfiles directory"
        $runfilesDir = "${PSCommandPath}.runfiles"
    # todo: RUNFILES_MANIFEST support
    } else {
        # Otherwise, we are probably a program within another program's
        # runfiles tree. Search up to find the runfiles directory.
        $currentDir = Split-Path -Path "$PSCommandPath" -Parent
        while ($currentDir -ne "") {
            if ($currentDir -like "*.runfiles") {
                $runfilesDir = $currentDir
                Write-Verbose "Runfiles found via parent directory search"
                break
            }
            $currentDir = Split-Path -Path "$currentDir" -Parent
        }
    }
    if ($runfilesDir -eq $null) {
        throw "ERROR: Unable to find runfiles directory for $PSCommandPath"
    }
    Write-Verbose "Runfiles: $runfilesDir"

    # runfiles-relative path to venv within runfiles
    $runfilesVenvRoot = Split-Path -Parent (Split-Path -Parent $PYTHON_BINARY)

    if ([string]::IsNullOrEmpty($Env:RULES_PYTHON_EXTRACT_ROOT)) {
        $tempDirRoot = $Env:TEMP
        $tempDirName = "rp-venv-" + (New-Guid).ToString()
        $venvRoot = Join-Path -Path $tempDirRoot -ChildPath $tempDirName
        $cleanupVenv = $bootstrapVerbose -eq "1"
    } else {
        $cleanupVenv = $false
        $venvRoot = Join-Path -Path $Env:RULES_PYTHON_EXTRACT_ROOT -Child $runfilesVenvRoot
        mkdir -p $venvRoot
    }

    # todo: Remove this debug path
    if ($true) {
        $venvRoot = "C:\tempvenv"
        if (Test-Path $venvRoot) {
            Remove-Item -Path $venvRoot -Recurse -Force
        }
        $cleanupVenv = $false
    }
    Write-Verbose "Venv root: $venvRoot"

    $pyBasename = Split-Path $PYTHON_BINARY_ACTUAL -Leaf
    $pyExe = "$venvRoot\Scripts\$pyBasename"
    # If RULES_PYTHON_EXTRACT_ROOT is set, then the venv may already exist,
    # so no need to recreate it. We test for pyvenv.cfg because that's the
    # last thing created in the venv.
    $createVenv = (-not (Test-Path "$venvRoot\pyvenv.cfg"))
    $createVenv = $true # todo: remove debug
    if ($createVenv) {
        Write-Verbose "venv doesn't exist (or incomplete), creating it"
        mkdir -p $venvRoot\Scripts | Out-Null
        newSymlink -Path $pyExe -Target "$runfilesDir\$PYTHON_BINARY_ACTUAL"

        $pyActualDir = Split-Path -Parent "$runfilesDir\$PYTHON_BINARY_ACTUAL"

        # Mimic Python venv behavior: it copies the dll's locally because
        # Python searches for the dll's in the sys.executable location.
        $dlls = Get-ChildItem -Path $pyActualDir -Filter "*.dll"
        foreach ($dll in $dlls) {
            $basename = Split-Path $dll -Leaf
            newSymlink -Path $venvRoot\Scripts\$basename -Target $dll
        }

        newSymlink -Path $venvRoot\Lib -Target $runfilesVenvRoot\Lib
        # While getpath.py appears to have code paths that should allow
        # the home key to be unset, empirical testing doesn't agree, so
        # we just have to create and set it at runtime.
        # NOTE: Creating the pyvenv.cfg file is done last; it acts as the
        # marker to indicate creation of the venv is complete.
        "home = $pyActualDir" >> "$venvRoot\pyvenv.cfg"
    }

    # PowerShell doesn't have a '-x' check like Unix, so just check existence.
    if (-not (Test-Path $pyExe)) {
        throw "ERROR: Python interpreter not found: $pyExe"
    }

    $stage2Path = Join-Path $runfilesDir $STAGE2_BOOTSTRAP

    # --- Prepare Arguments and Environment ---
    $pyEnv = @{}
    $argv = @()

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
        $argv.Add("-XRULES_PYTHON_ZIP_DIR=$zip_dir")
    }

    if (-not [string]::IsNullOrEmpty($Env:RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS)) {
        # Split the string into an array of args (basic space splitting)
        $argsFromEnv = $Env:RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS -split ' '
        foreach ($arg in $argsFromEnv) {
            Write-Verbose "Adding interpreter arg from env: $arg"
            $argv.Add($arg)
        }
        Remove-Item Env:\RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS
    }

    # Set after custom user env vars
    $pyEnv["RUNFILES_DIR"] = $runfilesDir

    # Combine all arguments for the python process
    $argv += $INTERPRETER_ARGS_FROM_TARGET
    $argv += $stage2Path # Add the stage2 script
    $argv += $args # Add arguments passed to this bootstrap script ($@)

    # IMPORTANT: Modifications to $Env affect the caller because powershell
    # scripts are run *in-process*. Thus we have to manually save/restore state.
    $origEnv = @{}
    if (Test-Path Env:RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS) {
        $origEnv["RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS"] = $Env:RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS
        Remove-Item Env:\RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS
    }
    foreach ($key in $pyEnv.Keys) {
        if (Test-Path Env:$key) {
            $origEnv[$key] = Get-Item "Env:$key"
        } else {
            $origEnv[$key] = $null
        }
    }
    $pyEnv["RULES_PYTHON_BOOTSTRAP_VERBOSE"] = ""  # todo: remove debug
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
        # It connects the standards streams, preserves the exit code, etc.
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
  if ($cleanupVenv) {
    Write-Verbose "Deleting venv: $venvRoot"
    Remove-Item -Path $venvRoot -Recurse -Force
  }
}

# The script should exit within the try block or via error preference.
# This is just a fallback.
exit 0
