$ErrorActionPreference = 'Stop'
$Script = Join-Path $HOME '.codexflow\current\codexflow.py'
if (-not (Test-Path $Script)) { throw 'CodexFlow is not installed. Run scripts\install.ps1 first.' }
if (Get-Command py -ErrorAction SilentlyContinue) { py -3 $Script doctor; exit $LASTEXITCODE }
python $Script doctor
exit $LASTEXITCODE
