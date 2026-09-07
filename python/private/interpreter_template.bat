@echo off
SETLOCAL ENABLEEXTENSIONS
SETLOCAL ENABLEDELAYEDEXPANSION

set "PYTHON_EXE_RUNFILES_PATH=%python_exe_runfiles_path%"
set "TF_WIN=!PYTHON_EXE_RUNFILES_PATH:/=\!"
set "MAIN_BIN="

REM --- Resolve manifest file ---
set "MF="
if defined RUNFILES_MANIFEST_FILE (
  set "MF=%RUNFILES_MANIFEST_FILE:/=\%"
)
if not defined MF (
  if exist "%~f0.runfiles_manifest" (
    set "MF=%~f0.runfiles_manifest"
  ) else if exist "%~dpn0.runfiles_manifest" (
    set "MF=%~dpn0.runfiles_manifest"
  ) else if exist "%~f0.runfiles\MANIFEST" (
    set "MF=%~f0.runfiles\MANIFEST"
  ) else if exist "%~dpn0.runfiles\MANIFEST" (
    set "MF=%~dpn0.runfiles\MANIFEST"
  )
)

if "%RUNFILES_MANIFEST_ONLY%" neq "1" (
  if defined RUNFILES_DIR (
    if exist "%RUNFILES_DIR%\!TF_WIN!" (
      set "MAIN_BIN=%RUNFILES_DIR%\!TF_WIN!"
    )
  )
  if not defined MAIN_BIN (
    if exist "%~f0.runfiles\!TF_WIN!" (
      set "MAIN_BIN=%~f0.runfiles\!TF_WIN!"
    ) else if exist "%~dpn0.runfiles\!TF_WIN!" (
      set "MAIN_BIN=%~dpn0.runfiles\!TF_WIN!"
    )
  )
)

if not defined MAIN_BIN if defined MF (
  if exist "!MF!" (
    for /F "usebackq tokens=1* delims= " %%a in ("!MF!") do (
      if "%%a"=="!PYTHON_EXE_RUNFILES_PATH!" (
        set "MAIN_BIN=%%b"
        goto :found_bin
      )
    )
  )
)
:found_bin

if not defined MAIN_BIN (
  echo>&2 ERROR: interpreter executable not found: !PYTHON_EXE_RUNFILES_PATH!
  exit /b 1
)

set "MAIN_BIN=!MAIN_BIN:/=\!"
if not exist "!MAIN_BIN!" (
  echo>&2 ERROR: interpreter executable not found: !MAIN_BIN!
  echo>&2 (from !PYTHON_EXE_RUNFILES_PATH!)
  exit /b 1
)

rem Determine PYTHONHOME (installation prefix containing Lib).
rem We must set PYTHONHOME to point to the runfiles directory because runfiles
rem provides the unified merge of source files and generated files needed by
rem the standard library and runtime.
rem In Windows layouts, Lib is typically sibling to python.exe (%%~dpdLib).
rem In hierarchical layouts, Lib may be in the parent directory (%%~dpd..\Lib).
for %%d in ("!MAIN_BIN!") do (
  if exist "%%~dpdLib" (
    set "PYTHONHOME=%%~dpd"
  ) else if exist "%%~dpd..\Lib" (
    for %%p in ("%%~dpd..") do set "PYTHONHOME=%%~fp"
  ) else (
    set "PYTHONHOME=%%~dpd"
  )
)
if "!PYTHONHOME:~-1!"=="\" set "PYTHONHOME=!PYTHONHOME:~0,-1!"

"!MAIN_BIN!" %*
exit /b !ERRORLEVEL!
