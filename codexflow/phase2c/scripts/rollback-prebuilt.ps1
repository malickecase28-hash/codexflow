param(
    [string]$InstallDir
)

$ErrorActionPreference = "Stop"

if (-not $InstallDir) {
    if (Test-Path "F:\") {
        $InstallDir = "F:\CodexFlow"
    } else {
        $InstallDir = Join-Path $HOME ".codexflow\runtime"
    }
}

$installLeaf = Split-Path -Leaf $InstallDir
$InstallRoot = if ($installLeaf -ieq "bin") {
    Split-Path -Parent $InstallDir
} else {
    $InstallDir
}

$currentPointer = Join-Path $InstallRoot "current.txt"
$previousPointer = Join-Path $InstallRoot "previous.txt"
if (-not (Test-Path -LiteralPath $currentPointer -PathType Leaf)) {
    throw "CodexFlow current runtime pointer is missing: $currentPointer"
}
if (-not (Test-Path -LiteralPath $previousPointer -PathType Leaf)) {
    throw "CodexFlow previous runtime pointer is missing: $previousPointer"
}

$current = (Get-Content -Raw $currentPointer).Trim()
$previous = (Get-Content -Raw $previousPointer).Trim()
if (-not $current -or -not $previous) {
    throw "CodexFlow current/previous runtime pointer is empty"
}
if ($current -eq $previous) {
    throw "CodexFlow current and previous pointers resolve to the same release"
}

$targetBin = Join-Path (Join-Path (Join-Path $InstallRoot "releases") $previous) "bin"
$required = @(
    "codex.exe",
    "codexflow.exe",
    "codex-code-mode-host.exe",
    "codexflow-supervisor.exe"
)
foreach ($name in $required) {
    $path = Join-Path $targetBin $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Previous CodexFlow runtime is incomplete: missing $path"
    }
}

& (Join-Path $targetBin "codex.exe") --version
if ($LASTEXITCODE -ne 0) { throw "Previous codex.exe smoke test failed" }
& (Join-Path $targetBin "codexflow.exe") --version
if ($LASTEXITCODE -ne 0) { throw "Previous codexflow.exe smoke test failed" }
& (Join-Path $targetBin "codexflow-supervisor.exe") --version
if ($LASTEXITCODE -ne 0) { throw "Previous codexflow-supervisor.exe smoke test failed" }
& (Join-Path $targetBin "codexflow.exe") setup --force
if ($LASTEXITCODE -ne 0) { throw "Previous CodexFlow setup failed" }

$previousTmp = "$previousPointer.tmp"
$current | Set-Content -Encoding ASCII $previousTmp
Move-Item -Force $previousTmp $previousPointer

$currentTmp = "$currentPointer.tmp"
$previous | Set-Content -Encoding ASCII $currentTmp
Move-Item -Force $currentTmp $currentPointer

Write-Host "CodexFlow runtime rollback complete:"
Write-Host "  current  -> $previous"
Write-Host "  previous -> $current"
