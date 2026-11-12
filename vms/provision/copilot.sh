#!/bin/bash
# Provisioning script of the GitHub Copilot CLI preset. It runs as root in provision mode,
# with internet access, before the instance's first agent boot. A failed run is retried on
# the next start, so every step must be safe to run again.
set -euo pipefail

# Copilot CLI (/usr/bin/copilot), from npm with its native linux-arm64 binary; Node comes
# with the base image. Sign in with /login: it shows a code for github.com/login/device.
# Without a keyring, Copilot offers to keep the token in ~/.copilot/config.json.
npm install -g @github/copilot

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor start the instance's command (copilot) as ubuntu.
sudo -u ubuntu -H bash -euo pipefail -s <<'SCRIPT'
cd ~

# Copilot starts in ~/workspace.
mkdir -p workspace
SCRIPT
