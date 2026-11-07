#!/bin/bash
# Provisioning script of the OpenCode preset. It runs as root in provision mode, with
# internet access, before the instance's first agent boot. A failed run is retried on the
# next start, so every step must be safe to run again.
set -euo pipefail

# OpenCode (/usr/local/bin/opencode), from npm; Node comes with the base image.
npm install -g opencode-ai

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor start the instance's command (opencode) as ubuntu.
sudo -u ubuntu -H bash -euo pipefail -s <<'EOF'
cd ~

# OpenCode starts in ~/workspace.
mkdir -p workspace
EOF
