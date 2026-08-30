@echo off
setlocal
where py >nul 2>nul && (py -3 "%USERPROFILE%\.codexflow\current\codexflow.py" %* & exit /b %ERRORLEVEL%)
python "%USERPROFILE%\.codexflow\current\codexflow.py" %*
