#Requires -Version 5.1 # Minimum version for Expand-Archive, symbolic links might need newer OS/admin rights

# Equivalent to set -e
$ErrorActionPreference = 'Stop'

# Equivalent to set -x (for debugging)
# Set-PSDebug -Trace 1
# Or use Write-Verbose statements throughout the script with $VerbosePreference = 'Continue'

# Check if verbose mode is requested (using environment variable)
if ($Env:RULES_PYTHON_BOOTSTRAP_VERBOSE) {
    # Enable detailed script execution tracing
    Set-PSDebug -Trace 2
    $VerbosePreference = 'Continue' # Also enable Write-Verbose
    Write-Verbose "Verbose mode enabled."
}

# --- Variable Definitions ---
# runfiles-relative path
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
$INTERPRETER_ARGS_FROM_TARGET = @(
    %interpreter_args%
)

$RUNFILES_DIR = $null
$zip_dir = $null
$use_exec = $true # PowerShell doesn't have exec, so we control flow differently. True means we try to mimic exec behavior at the end.

# --- Main Logic ---

try { # Wrap main logic for potential cleanup using finally (mimics trap EXIT)

    if ($IS_ZIPFILE -eq "1") {
        Write-Verbose "Processing as Zipfile..."
        # Create a temporary directory (more robust than mktemp)
        $zip_dir = Join-Path $Env:TEMP ([System.IO.Path]::GetRandomFileName())
        New-Item -ItemType Directory -Path $zip_dir -Force | Out-Null
        Write-Verbose "Created temporary directory for zip extraction: $zip_dir"

        $scriptPath = $MyInvocation.MyCommand.Path

        # Extract the archive. PowerShell's Expand-Archive might fail on self-extracting scripts like this.
        # This part might need an external unzip tool or different packaging approach on Windows.
        Write-Verbose "Attempting to extract '$scriptPath' to '$zip_dir'"
        try {
            # Suppress progress bar (-q equivalent), redirect errors to null (2>/dev/null equivalent), ignore exit code
            Expand-Archive -Path $scriptPath -DestinationPath $zip_dir -Force -ErrorAction SilentlyContinue *>$null
        } catch {
            Write-Warning "Expand-Archive might have encountered issues (potentially due to script prelude). Continuing..."
        }

        $RUNFILES_DIR = Join-Path $zip_dir "runfiles"
        if (-not (Test-Path $RUNFILES_DIR -PathType Container)) {
            Write-Error "Runfiles dir not found after extraction: '$RUNFILES_DIR'. Zip extraction likely failed."
            Write-Error "Run with RULES_PYTHON_BOOTSTRAP_VERBOSE=1 (set environment variable) to aid debugging."
            exit 1 # Equivalent to exit 1 in bash
        }
        Write-Verbose "Runfiles directory set to: $RUNFILES_DIR"
        $use_exec = $false # Cannot 'exec' if cleanup is needed

    } else {
        Write-Verbose "Processing as non-Zipfile..."
        # Function to find Runfiles Root (PowerShell adaptation)
        function Find-RunfilesRoot {
            param(
                [string]$ScriptPath
            )

            if (-not [string]::IsNullOrEmpty($Env:RUNFILES_DIR)) {
                Write-Verbose "Runfiles root from ENV:RUNFILES_DIR: $Env:RUNFILES_DIR"
                return $Env:RUNFILES_DIR
            }

            $manifestFile = $Env:RUNFILES_MANIFEST_FILE
            if ($manifestFile -like '*.runfiles_manifest') {
                 $runfilesDir = $manifestFile -replace '\.runfiles_manifest$', '.runfiles'
                 Write-Verbose "Runfiles root from manifest (1): $runfilesDir"
                 return $runfilesDir
            }
            if ($manifestFile -like '*.runfiles\MANIFEST') { # Adjusted for Windows paths
                 $runfilesDir = $manifestFile -replace '\.runfiles\\MANIFEST$', '.runfiles' # Adjusted for Windows paths
                 Write-Verbose "Runfiles root from manifest (2): $runfilesDir"
                 return $runfilesDir
            }

            # Resolve script path to absolute if needed
            if (-not ([System.IO.Path]::IsPathRooted($ScriptPath))) {
                $ScriptPath = Join-Path $PWD $ScriptPath
            }
            $currentPath = $ScriptPath

            while ($true) {
                $moduleSpace = "$currentPath.runfiles"
                if (Test-Path $moduleSpace -PathType Container) {
                    Write-Verbose "Runfiles root found (module space): $moduleSpace"
                    return $moduleSpace
                }

                if ($currentPath -like '*.runfiles\*') {
                     $runfilesDir = $currentPath -replace '(\.runfiles\\).*', '$1' # Adjusted for Windows paths and regex
                     $runfilesDir = $runfilesDir.TrimEnd('\') # Remove trailing slash if any
                     Write-Verbose "Runfiles root found (parent): $runfilesDir"
                     return $runfilesDir
                }

                # Check if it's a symlink (ReparsePoint in Windows)
                $item = Get-Item $currentPath -ErrorAction SilentlyContinue
                if ($item -and ($item.Attributes -match 'ReparsePoint')) {
                     # Note: Getting symlink target reliably can be complex, especially across different Windows versions/FS types.
                     # This might need adjustment based on the specific environment.
                     # Using Resolve-Path might work for some cases. Let's assume direct Get-Item might give target or need more logic.
                     # For simplicity, this part is less robust than the bash `readlink` loop.
                     Write-Warning "Symlink resolution in Find-RunfilesRoot might be incomplete."
                     # Attempt basic resolution (might not work for all link types)
                     try {
                         $resolved = Resolve-Path -LiteralPath $currentPath
                         if ($resolved.ProviderPath -ne $currentPath) {
                            $currentPath = $resolved.ProviderPath
                            continue # Continue loop with resolved path
                         }
                     } catch {
                         Write-Warning "Could not resolve symlink '$currentPath'"
                     }
                }

                # If not a symlink or resolution failed/didn't change, stop searching upwards this way.
                break
            }

            Write-Error "Unable to find runfiles directory for '$ScriptPath'"
            exit 1
        }
        $RUNFILES_DIR = Find-RunfilesRoot $MyInvocation.MyCommand.Path
        Write-Verbose "Runfiles directory set to: $RUNFILES_DIR"
    }

    # Function to find Python Interpreter (PowerShell adaptation)
    function Find-PythonInterpreter {
        param(
            [string]$RunfilesRoot,
            [string]$InterpreterPath
        )
        if ([System.IO.Path]::IsPathRooted($InterpreterPath)) {
            # Absolute path
            Write-Verbose "Python interpreter is absolute path: $InterpreterPath"
            return $InterpreterPath
        } elseif ($InterpreterPath -like '*\*' -or $InterpreterPath -like '*/*') {
            # Runfiles-relative path (handle both / and \)
            $fullPath = Join-Path $RunfilesRoot $InterpreterPath
            Write-Verbose "Python interpreter is runfiles relative: $fullPath"
            return $fullPath
        } else {
            # Single word - search PATH
            Write-Verbose "Searching for Python interpreter '$InterpreterPath' in PATH..."
            $found = Get-Command $InterpreterPath -ErrorAction SilentlyContinue
            if ($found) {
                Write-Verbose "Found in PATH: $($found.Source)"
                return $found.Source
            } else {
                Write-Error "Python interpreter '$InterpreterPath' not found in PATH."
                exit 1
            }
        }
    }

    $python_exe = Find-PythonInterpreter $RUNFILES_DIR $PYTHON_BINARY

    # Recreate venv symlink logic for Zipfiles or RECREATE_VENV_AT_RUNTIME
    if (($IS_ZIPFILE -eq "1") -or ($RECREATE_VENV_AT_RUNTIME -eq "1")) {
        Write-Verbose "Recreating venv structure or links..."
        $venv_dir_to_create = $null

        if ($IS_ZIPFILE -eq "1") {
            Write-Verbose "Zipfile mode: Creating symlink for Python binary."
            $use_exec = $false # Cannot 'exec' if cleanup is needed (zip_dir)
             # Ensure python_exe is under runfiles dir
             if (-not $python_exe.StartsWith($RUNFILES_DIR)) {
                 Write-Error "ERROR: Program's venv binary not under runfiles: $python_exe"
                 exit 1
             }

        } else { # RECREATE_VENV_AT_RUNTIME == "1"
            Write-Verbose "Recreate venv mode: Setting up venv directory."
             if ($Env:RULES_PYTHON_EXTRACT_ROOT) {
                 $venv_base = Join-Path $Env:RULES_PYTHON_EXTRACT_ROOT (Split-Path (Split-Path $PYTHON_BINARY -Parent) -Parent)
                 New-Item -ItemType Directory -Path $venv_base -Force | Out-Null
                 $venv_dir_to_create = $venv_base
                 Write-Verbose "Using persistent venv dir: $venv_dir_to_create"
                 # $use_exec remains $true if using persistent location
             } else {
                 $venv_dir_to_create = Join-Path $Env:TEMP ([System.IO.Path]::GetRandomFileName())
                 New-Item -ItemType Directory -Path $venv_dir_to_create -Force | Out-Null
                 Write-Verbose "Using temporary venv dir: $venv_dir_to_create"
                 $use_exec = $false # Cannot 'exec' if temp cleanup is needed
             }
        }

        # Determine the actual Python binary target for the symlink
        $symlink_target = $null
        if ([System.IO.Path]::IsPathRooted($PYTHON_BINARY_ACTUAL)) {
            $symlink_target = $PYTHON_BINARY_ACTUAL
        } elseif ($PYTHON_BINARY_ACTUAL -like '*\*' -or $PYTHON_BINARY_ACTUAL -like '*/*') {
            $symlink_target = Join-Path $RUNFILES_DIR $PYTHON_BINARY_ACTUAL
        } else {
            $found = Get-Command $PYTHON_BINARY_ACTUAL -ErrorAction SilentlyContinue
            if ($found) {
                $symlink_target = $found.Source
            } else {
                Write-Error "ERROR: Python target binary '$PYTHON_BINARY_ACTUAL' not found in PATH."
                exit 1
            }
        }
        Write-Verbose "Symlink target resolved to: $symlink_target"

        # Define the path for the symlink ($python_exe might change in recreate mode)
        $symlink_path = $python_exe
        if ($RECREATE_VENV_AT_RUNTIME -eq "1") {
            # In recreate mode, the link goes inside the *new* venv dir
            $python_exe = Join-Path $venv_dir_to_create "bin" (Split-Path $PYTHON_BINARY_ACTUAL -Leaf)
            $symlink_path = $python_exe # Update python_exe to the new path
             Write-Verbose "Python executable path updated to: $python_exe"
        }

        # Create parent directory for the symlink if needed
        $symlink_parent_dir = Split-Path $symlink_path -Parent
        if (-not (Test-Path $symlink_parent_dir)) {
            Write-Verbose "Creating parent directory for symlink: $symlink_parent_dir"
            New-Item -ItemType Directory -Path $symlink_parent_dir -Force | Out-Null
        }

        # Create the symlink to the Python executable
        # NOTE: Creating symlinks might require Administrator privileges or Developer Mode enabled on Windows.
        if (Test-Path $symlink_path) {
            Write-Verbose "Symlink path '$symlink_path' already exists. Removing."
            Remove-Item -LiteralPath $symlink_path -Force -Recurse -ErrorAction SilentlyContinue
        }
        Write-Verbose "Creating symlink from '$symlink_path' to '$symlink_target'"
        try {
            New-Item -ItemType SymbolicLink -Path $symlink_path -Target $symlink_target -ErrorAction Stop | Out-Null
        } catch {
            Write-Error "Failed to create symlink '$symlink_path' -> '$symlink_target'. Error: $($_.Exception.Message)"
            Write-Warning "Creating symlinks might require Administrator privileges or Developer Mode enabled on Windows."
            exit 1
        }

        # If recreating venv, link other necessary files/dirs
        if ($RECREATE_VENV_AT_RUNTIME -eq "1") {
            $runfiles_venv_root = Join-Path $RUNFILES_DIR (Split-Path (Split-Path $PYTHON_BINARY -Parent) -Parent)
            Write-Verbose "Linking venv components from $runfiles_venv_root to $venv_dir_to_create"

            # Link pyvenv.cfg
            $cfg_link = Join-Path $venv_dir_to_create "pyvenv.cfg"
            $cfg_target = Join-Path $runfiles_venv_root "pyvenv.cfg"
            if (-not (Test-Path $cfg_link)) {
                 Write-Verbose "Linking '$cfg_link' -> '$cfg_target'"
                try { New-Item -ItemType SymbolicLink -Path $cfg_link -Target $cfg_target -ErrorAction Stop | Out-Null } catch { Write-Error "Failed to link pyvenv.cfg: $($_.Exception.Message)"; exit 1 }
            }

             # Link lib directory
             # Note: Linking directories (Junctions/Symlinks) also often needs permissions.
             $lib_link = Join-Path $venv_dir_to_create "Lib" # Common Windows name
             $lib_target = Join-Path $runfiles_venv_root "lib"
             if (-not (Test-Path $lib_link)) {
                 Write-Verbose "Linking '$lib_link' -> '$lib_target'"
                 try { New-Item -ItemType SymbolicLink -Path $lib_link -Target $lib_target -ErrorAction Stop | Out-Null } catch { Write-Error "Failed to link lib directory: $($_.Exception.Message)"; exit 1 }
             }
        }
    }

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

    # Build the command execution string/array
    # Using Start-Process allows setting environment variables more cleanly for the child process,
    # but makes capturing output/exit code slightly more complex and doesn't 'exec'.
    # Using '&' operator (Invoke-Expression style) is simpler but env var inheritance needs care.

    # Let's use the '&' approach, modifying the *current* process env temporarily for simplicity,
    # similar to how `env K=V cmd` works inline in bash.
    # Store original env values to restore them in 'finally' if needed (though maybe not strictly required if script exits)
    $original_env_values = @{}
    foreach ($key in $interpreter_env_vars.Keys) {
        if (Test-Path Env:\$key) { $original_env_values[$key] = $Env:$key } else { $original_env_values[$key] = $null }
        Set-Item -Path "Env:\$key" -Value $interpreter_env_vars[$key]
    }

    # Execute the python interpreter
    & $python_exe $final_python_args

    # Capture the exit code of the last command
    $lastExitCode = $LASTEXITCODE
    Write-Verbose "Python process exited with code: $lastExitCode"

    # Exit this script with the same code
    exit $lastExitCode

} finally {
    # --- Cleanup --- (Equivalent to trap EXIT)
    Write-Verbose "Executing final cleanup..."

     # Restore environment variables if they were changed
     if ($original_env_values) {
         foreach ($key in $original_env_values.Keys) {
             Write-Verbose "Restoring Env:\$key"
             if ($null -eq $original_env_values[$key]) { Remove-Item -Path "Env:\$key" -ErrorAction SilentlyContinue }
             else { Set-Item -Path "Env:\$key" -Value $original_env_values[$key] }
         }
     }

    # Clean up temporary directories if created and not verbose
    if (-not $Env:RULES_PYTHON_BOOTSTRAP_VERBOSE) {
        if ($zip_dir -and (Test-Path $zip_dir)) {
            Write-Verbose "Removing temporary zip directory: $zip_dir"
            Remove-Item -Recurse -Force -Path $zip_dir -ErrorAction SilentlyContinue
        }
        # Only remove temp venv if it wasn't the persistent one
        if (($RECREATE_VENV_AT_RUNTIME -eq "1") -and (-not $Env:RULES_PYTHON_EXTRACT_ROOT) -and $venv_dir_to_create -and (Test-Path $venv_dir_to_create)) {
             Write-Verbose "Removing temporary venv directory: $venv_dir_to_create"
             Remove-Item -Recurse -Force -Path $venv_dir_to_create -ErrorAction SilentlyContinue
        }
    } else {
        Write-Verbose "Skipping cleanup due to verbose mode."
         if ($zip_dir) { Write-Host "Temporary zip directory left at: $zip_dir"}
         if (($RECREATE_VENV_AT_RUNTIME -eq "1") -and (-not $Env:RULES_PYTHON_EXTRACT_ROOT) -and $venv_dir_to_create) { Write-Host "Temporary venv directory left at: $venv_dir_to_create"}
    }

    # Restore original error preference if needed (though script is exiting)
    # $ErrorActionPreference = 'Continue'
}

# The script should exit within the try block or via error preference.
# This is just a fallback.
exit 0
