#!/usr/bin/env sh
set -e
hook=".git/hooks/pre-push"
cat > "$hook" <<'EOF'
#!/usr/bin/env sh
exec sh scripts/check.sh
EOF
chmod +x "$hook"
echo "installed pre-push hook -> scripts/check.sh"
