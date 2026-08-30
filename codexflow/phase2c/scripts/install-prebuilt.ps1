param(
    [string]$Repository = "malickecase28-hash/codexflow",
    [string]$Tag,
    [string]$InstallDir,
    [switch]$AddToPath
)

$ErrorActionPreference = "Stop"

if (-not $InstallDir) {
    if (Test-Path "F:\") {
        $InstallDir = "F:\CodexFlow\bin"
    } else {
        $InstallDir = Join-Path $HOME ".codexflow\prebuilt"
    }
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

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Expand-Archive -Force -Path $zipPath -DestinationPath $InstallDir

    & (Join-Path $InstallDir "codexflow.exe") setup --force

    $launcherDir = Join-Path $HOME ".codexflow\bin"
    New-Item -ItemType Directory -Force -Path $launcherDir | Out-Null
    $launcher = Join-Path $launcherDir "codexflow.cmd"
    "@echo off`r`n`"$InstallDir\codexflow.exe`" %*`r`n" | Set-Content -Encoding ASCII $launcher

    if ($AddToPath) {
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        $parts = @($userPath -split ';' | Where-Object { $_ })
        if ($parts -notcontains $launcherDir) {
            [Environment]::SetEnvironmentVariable("Path", (($parts + $launcherDir) -join ';'), "User")
        }
    }

    Write-Host "Installed CodexFlow:"
    Write-Host "  $InstallDir"
    Write-Host "Launcher:"
    Write-Host "  $launcher"
    Write-Host "Stock codex PATH resolution was not changed."
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
