$ErrorActionPreference = 'Stop'
$Bin = Join-Path $HOME '.codexflow\bin'
$Current = Join-Path $HOME '.codexflow\current'
$CodexHome = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $HOME '.codex' }
$Profile = Join-Path $CodexHome 'codexflow.config.toml'
$Agents = Join-Path $CodexHome 'agents'

Remove-Item $Profile -Force -ErrorAction SilentlyContinue
Get-ChildItem $Agents -Filter 'trinity_*.toml' -ErrorAction SilentlyContinue | Remove-Item -Force
Remove-Item $Current -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $Bin -Recurse -Force -ErrorAction SilentlyContinue

$UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$Parts = @($UserPath -split ';' | Where-Object { $_ -and $_ -ne $Bin })
[Environment]::SetEnvironmentVariable('Path', ($Parts -join ';'), 'User')
Write-Host 'Removed CodexFlow profile, roles, launcher and PATH entry. Project .codexflow state was left untouched.'
