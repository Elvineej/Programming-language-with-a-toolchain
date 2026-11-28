#!/usr/bin/env sh
# Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
# SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
set -e
hook=".git/hooks/pre-push"
cat > "$hook" <<'EOF'
#!/usr/bin/env sh
exec sh scripts/check.sh
EOF
chmod +x "$hook"
echo "installed pre-push hook -> scripts/check.sh"
