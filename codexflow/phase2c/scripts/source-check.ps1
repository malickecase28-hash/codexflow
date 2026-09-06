param(
    [string]$TargetDir = "F:\codexflow-target",
    [switch]$RunTests
)

$ErrorActionPreference = "Stop"
$PhaseRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$Repo = Resolve-Path (Join-Path $PhaseRoot "..\..")
$CodexRs = Join-Path $Repo "codex-rs"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo is required"
}

New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null
$env:CARGO_TARGET_DIR = $TargetDir

Push-Location $CodexRs
try {
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo check -p codex-cli --bin codexflow
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    if ($RunTests) {
        cargo test -p codex-cli --bin codexflow
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
}
finally {
    Pop-Location
}

Write-Host "Source validation complete. No release build was performed."
