param(
    [string]$TargetDir = "F:\codexflow-target",
    [switch]$RunTests,
    [switch]$ReleaseBuild
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

    if ($ReleaseBuild) {
        Write-Warning "Release linking is intentionally opt-in. Prefer the CodexFlow GitHub prebuilt workflow."
        cargo build --release -p codex-cli --bin codex --bin codexflow
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
}
finally {
    Pop-Location
}

Write-Host "Phase 2B/2C source validation complete."
if (-not $ReleaseBuild) {
    Write-Host "No release build was performed."
}
