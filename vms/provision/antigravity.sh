#!/bin/bash
# Provisioning script of the Antigravity CLI preset, Google's successor of Gemini CLI. It runs
# as root in provision mode, with internet access, before the instance's first agent boot. A
# failed run is retried on the next start, so every step must be safe to run again.
set -euo pipefail

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor start the instance's command (agy) as ubuntu.
sudo -u ubuntu -H bash -euo pipefail -s <<'SCRIPT'
cd ~

# Antigravity CLI (~/.local/bin/agy), Google's installer; it checks the download's SHA-512.
# It signs in on first use: open the URL it prints, then paste the code back within about
# 30 seconds. Its updater isn't allowed in agent mode; update in maintenance mode.
curl -fsSL https://antigravity.google/cli/install.sh | bash

# agy starts in ~/workspace.
mkdir -p workspace
SCRIPT
