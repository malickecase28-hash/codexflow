param(
    [string]$Repository = "malickecase28-hash/codexflow",
    [string]$Tag,
    [string]$InstallDir,
    [switch]$AddToPath
)

$ErrorActionPreference = "Stop"

if (-not $InstallDir) {
    if (Test-Path "F:\") {
        $InstallDir = "F:\CodexFlow"
    } else {
        $InstallDir = Join-Path $HOME ".codexflow\runtime"
    }
}

# Backward compatibility with the Phase 2C default, which pointed directly at
# an installation bin directory. Phase 3C treats the parent as a versioned
# runtime root instead of overwriting a live bundle in place.
$installLeaf = Split-Path -Leaf $InstallDir
$InstallRoot = if ($installLeaf -ieq "bin") {
    Split-Path -Parent $InstallDir
} else {
    $InstallDir
}

$releaseApi = if ($Tag) {
    "https://api.github.com/repos/$Repository/releases/tags/$Tag"
} else {
    "https://api.github.com/repos/$Repository/releases/latest"
}

$headers = @{
    "User-Agent" = "CodexFlow-Installer"
    "Accept" = "application/vnd.github+json"
}
$release = Invoke-RestMethod -Headers $headers -Uri $releaseApi
$zipName = "codexflow-windows-x86_64.zip"
$shaName = "$zipName.sha256"
$zipAsset = $release.assets | Where-Object name -eq $zipName | Select-Object -First 1
$shaAsset = $release.assets | Where-Object name -eq $shaName | Select-Object -First 1
if (-not $zipAsset -or -not $shaAsset) {
    throw "Release does not contain $zipName and $shaName"
}

$tmp = Join-Path $env:TEMP ("codexflow-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $zipPath = Join-Path $tmp $zipName
    $shaPath = Join-Path $tmp $shaName
    Invoke-WebRequest -Headers $headers -Uri $zipAsset.browser_download_url -OutFile $zipPath
    Invoke-WebRequest -Headers $headers -Uri $shaAsset.browser_download_url -OutFile $shaPath

    $expected = ((Get-Content -Raw $shaPath).Trim() -split "\s+")[0].ToLowerInvariant()
    $actual = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expected -ne $actual) {
        throw "SHA-256 mismatch: expected $expected actual $actual"
    }

    $stage = Join-Path $tmp "stage"
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Expand-Archive -Force -Path $zipPath -DestinationPath $stage

    $required = @(
        "codex.exe",
        "codexflow.exe",
        "codex-code-mode-host.exe",
        "codexflow-supervisor.exe"
    )
    foreach ($name in $required) {
        $path = Join-Path $stage $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Incomplete CodexFlow runtime bundle: missing $name"
        }
    }

    $releaseName = if ($Tag) { $Tag } elseif ($release.tag_name) { $release.tag_name } else { "release-$($release.id)" }
    $safeReleaseName = [regex]::Replace($releaseName, '[^A-Za-z0-9._-]', '_')
    $releaseId = "$safeReleaseName-$($actual.Substring(0, 12))"
    $releasesDir = Join-Path $InstallRoot "releases"
    $candidateRoot = Join-Path $releasesDir $releaseId
    $candidateBin = Join-Path $candidateRoot "bin"

    New-Item -ItemType Directory -Force -Path $releasesDir | Out-Null
    if (Test-Path -LiteralPath $candidateRoot) {
        Remove-Item -Recurse -Force $candidateRoot
    }
    New-Item -ItemType Directory -Force -Path $candidateBin | Out-Null
    foreach ($name in $required) {
        Copy-Item -Force (Join-Path $stage $name) (Join-Path $candidateBin $name)
    }

    # Candidate smoke tests happen before current.txt is changed. A failed
    # candidate therefore cannot replace the known-good runtime.
    & (Join-Path $candidateBin "codex.exe") --version
    if ($LASTEXITCODE -ne 0) { throw "Candidate codex.exe smoke test failed" }
    & (Join-Path $candidateBin "codexflow.exe") --version
    if ($LASTEXITCODE -ne 0) { throw "Candidate codexflow.exe smoke test failed" }
    & (Join-Path $candidateBin "codexflow-supervisor.exe") --version
    if ($LASTEXITCODE -ne 0) { throw "Candidate codexflow-supervisor.exe smoke test failed" }

    # Install/update the CodexFlow profile from the validated candidate before
    # promotion. If setup fails, current.txt remains unchanged.
    & (Join-Path $candidateBin "codexflow.exe") setup --force
    if ($LASTEXITCODE -ne 0) { throw "Candidate CodexFlow setup failed" }

    $currentPointer = Join-Path $InstallRoot "current.txt"
    $previousPointer = Join-Path $InstallRoot "previous.txt"
    $oldCurrent = if (Test-Path -LiteralPath $currentPointer) {
        (Get-Content -Raw $currentPointer).Trim()
    } else {
        $null
    }

    if ($oldCurrent -and $oldCurrent -ne $releaseId) {
        $previousTmp = "$previousPointer.tmp"
        $oldCurrent | Set-Content -Encoding ASCII $previousTmp
        Move-Item -Force $previousTmp $previousPointer
    }

    $currentTmp = "$currentPointer.tmp"
    $releaseId | Set-Content -Encoding ASCII $currentTmp
    Move-Item -Force $currentTmp $currentPointer

    $launcherDir = Join-Path $HOME ".codexflow\bin"
    New-Item -ItemType Directory -Force -Path $launcherDir | Out-Null
    $launcher = Join-Path $launcherDir "codexflow.cmd"
    $launcherText = @"
@echo off
setlocal
set "CODEXFLOW_ROOT=$InstallRoot"
set /p "CODEXFLOW_RELEASE="<"$currentPointer"
if not defined CODEXFLOW_RELEASE (
  echo CodexFlow runtime pointer is empty. 1>&2
  exit /b 1
)
"$InstallRoot\releases\%CODEXFLOW_RELEASE%\bin\codexflow.exe" %*
"@
    $launcherText | Set-Content -Encoding ASCII $launcher

    if ($AddToPath) {
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        $parts = @($userPath -split ';' | Where-Object { $_ })
        if ($parts -notcontains $launcherDir) {
            [Environment]::SetEnvironmentVariable("Path", (($parts + $launcherDir) -join ';'), "User")
        }
    }

    Write-Host "Installed CodexFlow runtime candidate and promoted it atomically:"
    Write-Host "  $candidateBin"
    Write-Host "Current release:"
    Write-Host "  $releaseId"
    if ($oldCurrent -and $oldCurrent -ne $releaseId) {
        Write-Host "Previous release retained:"
        Write-Host "  $oldCurrent"
    }
    Write-Host "Runtime companions verified:"
    Write-Host "  codex-code-mode-host.exe"
    Write-Host "  codexflow-supervisor.exe"
    Write-Host "Launcher:"
    Write-Host "  $launcher"
    Write-Host "Stock codex PATH resolution was not changed."
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
