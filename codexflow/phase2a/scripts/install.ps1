$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot

function Find-Python {
    if ($null -ne (Get-Command py -ErrorAction SilentlyContinue)) { return ,@('py', '-3') }
    if ($null -ne (Get-Command python -ErrorAction SilentlyContinue)) { return ,@('python') }
    throw 'Python 3.10+ is required for this bootstrap. Install Python, then rerun install.ps1.'
}

$Py = Find-Python
if ($Py.Count -eq 2) {
    & $Py[0] $Py[1] (Join-Path $Root 'codexflow.py') install --source $Root
} else {
    & $Py[0] (Join-Path $Root 'codexflow.py') install --source $Root
}
if ($LASTEXITCODE -ne 0) { throw "CodexFlow install failed with exit code $LASTEXITCODE" }

$Bin = Join-Path $HOME '.codexflow\bin'
$UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$Parts = @($UserPath -split ';' | Where-Object { $_ })
if (-not ($Parts -contains $Bin)) {
    $NewPath = (($Parts + $Bin) -join ';')
    [Environment]::SetEnvironmentVariable('Path', $NewPath, 'User')
    Write-Host "Added $Bin to the user PATH. Open a new terminal before calling codexflow by name."
}
Write-Host 'Installed CodexFlow without modifying the stock codex executable or ~/.codex/config.toml.'
Write-Host 'Next: open a new terminal in your repo, run codexflow init, then codexflow doctor.'
