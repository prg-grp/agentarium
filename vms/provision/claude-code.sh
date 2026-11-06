#!/bin/bash
# Default provisioning script for Claude Code. It runs as root in provision mode, with
# internet access, before the instance's first agent boot. A failed run is retried on the
# next start, so every step must be safe to run again.
set -euo pipefail

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor start the instance's command (claude) as ubuntu.
sudo -u ubuntu -H bash -euo pipefail -s <<'EOF'
cd ~

# Claude Code (~/.local/bin/claude).
curl -fsSL https://claude.ai/install.sh | bash

# Claude starts in ~/workspace, already trusted. It needs its own folder, because Claude
# Code never saves trust for the home folder.
mkdir -p workspace
[ -f .claude.json ] || echo '{}' > .claude.json
jq '.projects["/home/ubuntu/workspace"].hasTrustDialogAccepted = true' .claude.json > .claude.json.new
mv .claude.json.new .claude.json
chmod 600 .claude.json
EOF
