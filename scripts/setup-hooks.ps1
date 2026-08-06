$hook = ".git/hooks/pre-push"
Set-Content -Path $hook -Value "#!/usr/bin/env sh`nexec sh scripts/check.sh" -Encoding utf8
Write-Output "installed pre-push hook -> scripts/check.sh"
