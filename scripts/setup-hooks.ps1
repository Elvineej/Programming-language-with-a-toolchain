# Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
# SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
$hook = ".git/hooks/pre-push"
Set-Content -Path $hook -Value "#!/usr/bin/env sh`nexec sh scripts/check.sh" -Encoding utf8
Write-Output "installed pre-push hook -> scripts/check.sh"
