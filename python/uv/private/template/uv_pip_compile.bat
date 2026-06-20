@echo off
if not defined BUILD_WORKSPACE_DIRECTORY exit /b 1
set "out=%BUILD_WORKSPACE_DIRECTORY%\{{src_out}}"
"{{args}}" --output-file "%out%" %*
