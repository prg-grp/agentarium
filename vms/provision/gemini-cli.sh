#!/bin/bash
# Provisioning script of the Gemini CLI preset. It runs as root in provision mode, with
# internet access, before the instance's first agent boot. A failed run is retried on the
# next start, so every step must be safe to run again.
set -euo pipefail

# Gemini CLI (/usr/local/bin/gemini), from npm; Node comes with the base image.
npm install -g @google/gemini-cli

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor start the instance's command (gemini) as ubuntu.
sudo -u ubuntu -H bash -euo pipefail -s <<'EOF'
cd ~

# Gemini starts in ~/workspace, already trusted.
mkdir -p workspace .gemini
[ -f .gemini/trustedFolders.json ] || echo '{}' > .gemini/trustedFolders.json
jq '.["/home/ubuntu/workspace"] = "TRUST_FOLDER"' .gemini/trustedFolders.json > .gemini/trustedFolders.json.new
mv .gemini/trustedFolders.json.new .gemini/trustedFolders.json
EOF
