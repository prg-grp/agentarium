#!/bin/bash
# Provisioning script of the Codex preset. It runs as root in provision mode, with internet
# access, before the instance's first agent boot. A failed run is retried on the next start,
# so every step must be safe to run again.
set -euo pipefail

# Codex (/usr/local/bin/codex), from npm; Node comes with the base image.
npm install -g @openai/codex

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor start the instance's command (codex) as ubuntu.
sudo -u ubuntu -H bash -euo pipefail -s <<'EOF'
cd ~

# Codex starts in ~/workspace, already trusted.
mkdir -p workspace .codex
grep -qsF '[projects."/home/ubuntu/workspace"]' .codex/config.toml ||
    printf '\n[projects."/home/ubuntu/workspace"]\ntrust_level = "trusted"\n' >> .codex/config.toml
EOF
