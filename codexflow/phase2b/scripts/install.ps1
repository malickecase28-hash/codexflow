param(
    [string]$TargetDir = "F:\codexflow-target"
)

$ErrorActionPreference = "Stop"
$PhaseRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$Repo = Resolve-Path (Join-Path $PhaseRoot "..\..")
$CodexRs = Join-Path $Repo "codex-rs"
$TuiFile = Join-Path $CodexRs "tui\src\app_server_session.rs"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo is required"
}

$Tui = Get-Content -Raw $TuiFile
if ($Tui -notmatch 'CODEXFLOW_PROJECT_ID') {
    $Anchor = @'
        );
        if self.history_support == ThreadHistorySupport::LegacyOnly {
'@
    $Replacement = @'
        );
        if params.project_id.is_none()
            && let Ok(project_id) = std::env::var("CODEXFLOW_PROJECT_ID")
        {
            let project_id = project_id.trim();
            if !project_id.is_empty() {
                params.project_id = Some(project_id.to_string());
            }
        }
        if self.history_support == ThreadHistorySupport::LegacyOnly {
'@
    if (-not $Tui.Contains($Anchor)) {
        throw "TUI project bridge anchor not found. Upstream source changed; do not patch automatically."
    }
    $Tui = $Tui.Replace($Anchor, $Replacement)
    Set-Content -NoNewline -Encoding UTF8 $TuiFile $Tui
    Write-Host "Applied CODEXFLOW_PROJECT_ID thread-start bridge."
}

$CodexHome = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $HOME ".codex" }
$AgentsDir = Join-Path $CodexHome "agents"
New-Item -ItemType Directory -Force -Path $AgentsDir | Out-Null
Get-ChildItem (Join-Path $PhaseRoot "roles\flow_*.toml") | ForEach-Object {
    Copy-Item -Force $_.FullName (Join-Path $AgentsDir $_.Name)
}

$God = Get-Content -Raw (Join-Path $PhaseRoot "prompts\GOD.md")
if ($God.Contains('"""')) {
    throw "GOD prompt contains TOML triple quotes"
}
$Profile = @"
developer_instructions = """
$($God.TrimEnd())
"""

[features]
multi_agent = true

[agents]
max_threads = 8
max_depth = 2
"@
New-Item -ItemType Directory -Force -Path $CodexHome | Out-Null
Set-Content -Encoding UTF8 (Join-Path $CodexHome "codexflow.config.toml") $Profile

if ($TargetDir) {
    New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null
    $env:CARGO_TARGET_DIR = $TargetDir
    Write-Host "Using CARGO_TARGET_DIR=$TargetDir"
}

Push-Location $CodexRs
try {
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo test -p codex-cli --bin codexflow
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo check -p codex-tui
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo build --release -p codex-cli --bin codex
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo build --release -p codex-cli --bin codexflow
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
    Pop-Location
}

$Release = Join-Path $env:CARGO_TARGET_DIR "release"
Write-Host ""
Write-Host "Built:"
Write-Host "  $(Join-Path $Release 'codex.exe')"
Write-Host "  $(Join-Path $Release 'codexflow.exe')"
Write-Host ""
Write-Host "Next:"
Write-Host "  $(Join-Path $Release 'codexflow.exe') project add TrinityR --root F:\TrinityR"
Write-Host "  cd F:\TrinityR"
Write-Host "  $(Join-Path $Release 'codexflow.exe')"
