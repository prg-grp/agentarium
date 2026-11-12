#!/bin/bash
# Provisioning script of the Hermes Agent preset. It runs as root in provision mode, with
# internet access, before the instance's first agent boot. A failed run is retried on the
# next start, so every step must be safe to run again.
set -euo pipefail

# The Node.js that Hermes installs for itself needs libatomic. The installer doesn't use
# sudo when it runs without prompts, so it is installed here.
apt-get update
apt-get install -y libatomic1 xz-utils

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor run `hermes dashboard --no-open --port 9119` as
# ubuntu and forwards port 9119 to your Mac: the chip in the terminal's header opens the
# dashboard. On the VM's loopback it needs no login. Set up the model provider, its key and
# the messaging gateway there, and allow the provider's domain in the network settings.
sudo -u ubuntu -H bash -euo pipefail -s <<'SCRIPT'
cd ~
mkdir -p workspace

# Hermes (~/.local/bin/hermes, ~/.hermes) with its own Python, uv and Node, from the
# official installer. Without prompts it skips the setup wizard; the dashboard covers it.
curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash -s -- --non-interactive

# The dashboard builds its web interface on its first start, which downloads packages from
# npm. Start it once now, while the internet is open, and stop it once it answers.
~/.local/bin/hermes dashboard --no-open --port 9119 &
dashboard=$!
for _ in $(seq 300); do
    if curl -sf -o /dev/null http://127.0.0.1:9119 || ! kill -0 "$dashboard" 2>/dev/null; then
        break
    fi
    sleep 2
done
if curl -sf -o /dev/null http://127.0.0.1:9119; then
    echo "The Hermes dashboard is ready."
else
    echo "The Hermes dashboard didn't start yet; its first start in agent mode builds it, which needs npm's domains."
fi
kill "$dashboard" 2>/dev/null || true
wait "$dashboard" 2>/dev/null || true
SCRIPT
