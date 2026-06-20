@echo off
if not defined BUILD_WORKSPACE_DIRECTORY exit /b 1
"{{args}}" %*
