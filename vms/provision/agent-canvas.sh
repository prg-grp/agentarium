#!/bin/bash
# Provisioning script of the Agent Canvas preset, OpenHands' web interface for coding agents.
# It runs as root in provision mode, with internet access, before the instance's first agent
# boot. A failed run is retried on the next start, so every step must be safe to run again.
set -euo pipefail

# Agent Canvas (/usr/bin/agent-canvas), from npm; Node and uv come with the base image. It
# runs its agent server directly in the VM, without Docker: the VM is the sandbox.
npm install -g @openhands/agent-canvas

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor start the instance's command (agent-canvas) as
# ubuntu and forwards port 8000 to your Mac: the chip in the terminal's header opens it. On
# the VM's loopback it signs the browser in by itself. Set up the model and its key there,
# and allow the provider's domain in the network settings.
sudo -u ubuntu -H bash -euo pipefail -s <<'SCRIPT'
cd ~
mkdir -p workspace

# Agent Canvas fetches its agent server with uvx when it starts. Start it once now, while
# the internet is open, so the server is in the uv cache, and stop it once it answers.
# Agent boots still look the server up on PyPI, which the preset's rules allow.
agent-canvas &
canvas=$!
for _ in $(seq 300); do
    if curl -sf -o /dev/null http://127.0.0.1:8000 || ! kill -0 "$canvas" 2>/dev/null; then
        break
    fi
    sleep 2
done
if curl -sf -o /dev/null http://127.0.0.1:8000; then
    echo "Agent Canvas is ready."
else
    echo "Agent Canvas didn't start yet; its first start in agent mode sets it up."
fi
kill "$canvas" 2>/dev/null || true
wait "$canvas" 2>/dev/null || true
SCRIPT
