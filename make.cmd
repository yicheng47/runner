@echo off
setlocal

set "RUNNER_TARGET=%~1"
if not defined RUNNER_TARGET set "RUNNER_TARGET=build"
if /i not "%RUNNER_TARGET%"=="build" if /i not "%RUNNER_TARGET%"=="run" goto usage

set "RUNNER_PROFILE="
if not "%~2"=="" (
    if /i not "%~2"=="--release" goto usage
    set "RUNNER_PROFILE=--release"
)
if not "%~3"=="" goto usage
if not defined CARGO_BUILD_JOBS set "CARGO_BUILD_JOBS=12"

where cargo >nul 2>nul
if errorlevel 1 (
    if defined CARGO_HOME (
        set "PATH=%CARGO_HOME%\bin;%PATH%"
    ) else (
        set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
    )
)

pushd "%~dp0"
if errorlevel 1 exit /b 1
cargo build --workspace %RUNNER_PROFILE%
if errorlevel 1 goto done
if /i "%RUNNER_TARGET%"=="run" cargo run -p runner-app %RUNNER_PROFILE%

:done
set "RUNNER_EXIT_CODE=%ERRORLEVEL%"
popd
exit /b %RUNNER_EXIT_CODE%

:usage
echo Usage: %~nx0 [build^|run] [--release]
exit /b 2
