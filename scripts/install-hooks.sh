#!/usr/bin/env sh
# Installs the pre-commit hook that runs scripts/quality.sh before every commit.
set -eu
cd "$(dirname "$0")/.."
printf '#!/usr/bin/env sh\nexec scripts/quality.sh\n' > .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
echo "pre-commit hook installed"
