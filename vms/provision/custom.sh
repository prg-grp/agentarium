#!/bin/bash
# Provisioning script for a custom agent. Fill in the steps marked TODO.
#
# It runs once as root in provision mode, with internet access, before the instance's
# first agent boot. A failed run is retried on the next start, so every step must be safe
# to run again. The script's output shows in the instance's terminal.
set -euo pipefail

# System packages, installed as root. For example:
# apt-get update && apt-get install -y --no-install-recommends ripgrep

# Everything below is set up for the unprivileged ubuntu user. In agent mode the app has the
# guest supervisor start the instance's command (set in the Agent step) as ubuntu, in
# ~/workspace by default.
sudo -u ubuntu -H bash -euo pipefail -s <<'EOF'
cd ~

# TODO: install your agent for the ubuntu user, for example into ~/.local/bin.
# curl -fsSL https://example.com/install.sh | bash

# The agent's working folder; shared folders are mounted below it.
mkdir -p workspace
EOF
