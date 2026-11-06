#!/bin/sh
# Cargo runner: ad-hoc sign the binary with the virtualization entitlement, then run it.
set -eu
codesign --force --sign - --entitlements "$(dirname "$0")/../app/agent-toolbox.entitlements" "$1"
exec "$@"
