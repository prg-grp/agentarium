#!/bin/bash
# Provisioning script for a custom agent. Fill in the steps marked TODO.
#
# It runs once as root in provision mode, with internet access, before the instance's
# first agent boot. A failed run is retried on the next start, so every step must be safe
# to run again. The script's output shows in the instance's terminal.
set -euo pipefail

# System packages, installed as root. For example:
# apt-get update && apt-get install -y --no-install-recommends ripgrep

# Everything below is set up for the unprivileged ubuntu user, which runs the agent.
sudo -u ubuntu -H bash -euo pipefail -s <<'EOF'
cd ~

# TODO: install your agent for the ubuntu user, for example into ~/.local/bin.
# curl -fsSL https://example.com/install.sh | bash

# The agent's working folder; shared folders are mounted below it.
mkdir -p workspace

# Agent mode: the console logs in ubuntu, whose login starts the agent in ~/workspace.
# When it exits, the console logs in again and starts it anew. Maintenance and provision
# mode never run this. TODO: replace my-agent with your agent's command.
if ! grep -q '# Agent mode: run the agent on login.' .profile; then
    printf '\n# Agent mode: run the agent on login.\ngrep -qwE "agent_toolbox.mode=(maintenance|provision)" /proc/cmdline || { cd "$HOME/workspace" && exec my-agent; }\n' >> .profile
fi
EOF
