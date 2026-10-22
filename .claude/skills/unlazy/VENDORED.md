# Vendored copy of unlazy

- Source: https://github.com/Leonxlnx/unlazy
- Commit: 16671491f6679ad9378f52604d3bc2415b4120c7 (2026-09-03, source version 2.1.0, untagged)
- Licence: MIT (see `LICENSE`)
- Copied: the skill and its runtime files (`SKILL.md`, `SECURITY.md`, `README.md`,
  `agents/`, `references/`, `research/`, `scripts/`, `templates/`). Left out: upstream's
  `tests/`, `.github/`, `package.json`, `CHANGELOG.md`, `CONTRIBUTING.md`.
- Verified: upstream `npm test` passed at this commit on Node 22.

To update, clone upstream at the new commit, read the diff (the `scripts/` run shell code
from `CHECK:` lines once approved), copy the same files over this directory, and update
the commit above.

The optional Stop hook (`scripts/install-hooks.mjs`) is not installed; install it only on
purpose.
