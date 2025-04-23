# Equivalent to set -e
$ErrorActionPreference = 'Stop'

$Env:RULES_PYTHON_BOOTSTRAP_VERBOSE = "1"

# Check if verbose mode is requested (using environment variable)
if ($Env:RULES_PYTHON_BOOTSTRAP_VERBOSE -eq "1") {
    # Enable detailed script execution tracing
    ##Set-PSDebug -Trace 1
    $VerbosePreference = 'Continue' # Also enable Write-Verbose
    Write-Verbose "Verbose mode enabled."
}

##$STAGE2_BOOTSTRAP = "%stage2_bootstrap%"
$STAGE2_BOOTSTRAP = "_main\tests\bootstrap_impls\__run_binary_bootstrap_script_zip_no_test_bin_stage2_bootstrap.py"
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
$INTERPRETER_ARGS_FROM_TARGET = @(
    #"%interpreter_args%"
)

$RUNFILES_DIR = $null
$zip_dir = $null
$use_exec = $true # PowerShell doesn't have exec, so we control flow differently. True means we try to mimic exec behavior at the end.

Write-Verbose "python binary: $PYTHON_BINARY"
Write-Verbose "actual: $PYTHON_BINARY_ACTUAL"

# --- Main Logic ---

try { # Wrap main logic for potential cleanup using finally (mimics trap EXIT)

    Write-Verbose "Processing as non-Zipfile..."
    # Function to find Runfiles Root (PowerShell adaptation)
    
    $RUNFILES_DIR = Resolve-Path ".\bazel-bin\tests\bootstrap_impls\_run_binary_bootstrap_script_zip_no_test_bin.ps1.runfiles"
    Write-Verbose "Runfiles directory set to: $RUNFILES_DIR"

    # Function to find Python Interpreter (PowerShell adaptation)
    ##$python_exe = "C:\\Users\\richardlev\\AppData\\Local\\Microsoft\\WindowsApps\\PythonSoftwareFoundation.Python.3.10_qbz5n2kfra8p0\\python.exe"
    $python_exe = Join-Path $RUNFILES_DIR $PYTHON_BINARY

    # --- Interpreter Check ---
    Write-Verbose "Final check for Python executable: $python_exe"
    if (-not (Test-Path $python_exe)) {
        Write-Error "ERROR: Python interpreter not found: $python_exe"
        # Attempt to list parent dir content for debugging
        Get-ChildItem (Split-Path $python_exe -Parent) -ErrorAction SilentlyContinue
        exit 1
    }
    # PowerShell doesn't have a direct '-x' check like Unix. Get-Command can sometimes help verify executability.
    # We'll rely on the OS to fail if it's not runnable.

    $stage2_bootstrap_path = Join-Path $RUNFILES_DIR $STAGE2_BOOTSTRAP

    # --- Prepare Arguments and Environment ---
    $interpreter_env_vars = @{} # Use a hashtable for environment variables
    $interpreter_args_list = [System.Collections.Generic.List[string]]::new()

    # Handle PYTHONSAFEPATH (equivalent logic)
    if (-not (Test-Path Env:\PYTHONSAFEPATH)) {
        # If PYTHONSAFEPATH is not set at all in the environment, default it to 1
        $interpreter_env_vars['PYTHONSAFEPATH'] = '1'
        Write-Verbose "Setting PYTHONSAFEPATH=1"
    } else {
         # If it is set (even to empty string), pass its value through
         $interpreter_env_vars['PYTHONSAFEPATH'] = $Env:PYTHONSAFEPATH
         Write-Verbose "Passing through existing PYTHONSAFEPATH value: '$($Env:PYTHONSAFEPATH)'"
    }


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

    # --- Execute Python ---
    # Set Runfiles Dir for the child process
    $Env:RUNFILES_DIR = $RUNFILES_DIR
    Write-Verbose "Exporting RUNFILES_DIR=$RUNFILES_DIR"

    # Combine all arguments for the python process
    $final_python_args = @()
    $final_python_args += $interpreter_args_list # Add args derived in this script
    $final_python_args += $INTERPRETER_ARGS_FROM_TARGET # Add args from build target
    $final_python_args += $stage2_bootstrap_path # Add the stage2 script
    $final_python_args += $args # Add arguments passed to this bootstrap script ($@)

    Write-Verbose "Executing Python:"
    Write-Verbose "  Interpreter: $python_exe"
    Write-Verbose "  Arguments: $($final_python_args -join ' ')"
    if ($interpreter_env_vars.Count -gt 0) {
        Write-Verbose "  With additional environment:"
        $interpreter_env_vars.GetEnumerator() | ForEach-Object { Write-Verbose "    $($_.Key)=$($_.Value)" }
    }

    # Build the command execution string/array Using Start-Process allows
    # setting environment variables more cleanly for the child process, but
    # makes capturing output/exit code slightly more complex and doesn't
    # 'exec'.  Using '&' operator (Invoke-Expression style) is simpler but env
    # var inheritance needs care.

    # Let's use the '&' approach, modifying the *current* process env
    # temporarily for simplicity, similar to how `env K=V cmd` works inline in
    # bash.  Store original env values to restore them in 'finally' if needed
    # (though maybe not strictly required if script exits)
    ##$original_env_values = @{}
    ##foreach ($key in $interpreter_env_vars.Keys) {
    ##    if (Test-Path Env:\$key) {
    ##        $original_env_values[$key] = $Env:$key
    ##    } else { $original_env_values[$key] = $null }
    ##    Set-Item -Path "Env:\$key" -Value $interpreter_env_vars[$key]
    ##}

    $Env:RULES_PYTHON_BOOTSTRAP_VERBOSE = ""
    # Execute the python interpreter
    & $python_exe $final_python_args

    # Capture the exit code of the last command
    $lastExitCode = $LASTEXITCODE
    Write-Verbose "Python process exited with code: $lastExitCode"

    # Exit this script with the same code
    exit $lastExitCode

} finally {
  Write-Verbose "Finally block ran"
}

# The script should exit within the try block or via error preference.
# This is just a fallback.
exit 0
