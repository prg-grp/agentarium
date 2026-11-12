#!/bin/bash
# Provisioning script of the OpenClaw preset. It runs as root in provision mode, with
# internet access, before the instance's first agent boot. A failed run is retried on the
# next start, so every step must be safe to run again.
set -euo pipefail

# OpenClaw (/usr/bin/openclaw), from npm; Node comes with the base image. Newer npm only
# runs OpenClaw's install scripts when allowed; older npm warns about the flag and goes on.
npm install -g --allow-scripts=openclaw openclaw@latest

# Everything below is set up for the unprivileged ubuntu user, which runs the agent. In agent
# mode the app has the guest supervisor run `openclaw gateway run --port 18789` as ubuntu,
# and forwards port 18789 to your Mac: the chip in the terminal's header opens the Control
# UI. Don't install OpenClaw's own service (onboard --install-daemon): the supervisor keeps
# the gateway running, and ubuntu has no systemd user session.
sudo -u ubuntu -H bash -euo pipefail -s <<'SCRIPT'
cd ~
mkdir -p workspace

# The gateway refuses to start unless it is set up for local use.
openclaw config set gateway.mode local

# No gateway token: the gateway listens on the VM's loopback only, and only the app's
# forwarded port reaches it, so the Control UI opens without signing in. Set up models and
# channels there. Anyone who can reach the forwarded port can use the gateway, so keep
# that port on "This Mac" in the instance's network settings, or turn token auth back on
# (openclaw config set gateway.auth.mode token) before opening it to the local network.
openclaw config set gateway.auth.mode none
SCRIPT
