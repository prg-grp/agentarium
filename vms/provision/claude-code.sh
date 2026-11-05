#!/bin/bash
# Default provisioning script for Claude Code. It runs as root in provision mode, with
# internet access, before the instance's first agent boot. A failed run is retried on the
# next start, so every step must be safe to run again.
set -euo pipefail

# Everything below is set up for the unprivileged ubuntu user. No login shell, so the
# agent-mode hook in ~/.profile doesn't run here.
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

# Agent mode: the console logs in ubuntu, whose login runs Claude Code in ~/workspace.
# When it exits, the getty logs in again and starts it anew. Maintenance mode logs in
# root and provision mode logs in nobody, so this never runs there (also not for `su - ubuntu`).
if ! grep -q '# Agent mode: run Claude Code on login.' .profile; then
    printf '\n# Agent mode: run Claude Code on login.\ngrep -qwE "agent_toolbox.mode=(maintenance|provision)" /proc/cmdline || { cd "$HOME/workspace" && exec claude; }\n' >> .profile
fi
EOF
