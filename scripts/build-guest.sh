#!/usr/bin/env bash
# Downloads the guest kernel into target/guest/Image, an uncompressed arm64 Linux
# kernel (Ubuntu 24.04). The root filesystems are built by vms/rootfs/Makefile.
# The download is pinned by checksum and reused on later runs.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/guest"
WORK="$OUT/work"

KERNEL_URL="https://cloud-images.ubuntu.com/releases/noble/release-20261017/unpacked/ubuntu-24.04-server-cloudimg-arm64-vmlinuz-generic"
KERNEL_SHA256="353bab8eeaad9caf931dd85fd7a7bf8800d2669d759c0369ffaba7abc03a7aff"

mkdir -p "$WORK"

# Downloads $1 to $2 unless a file with checksum $3 is already there.
fetch() {
    local url="$1" file="$2" sha="$3"
    if [ -f "$file" ] && echo "$sha  $file" | shasum -a 256 -c --status; then
        return
    fi
    echo "Downloading $url"
    curl -fL --retry 3 -o "$file.part" "$url"
    if ! echo "$sha  $file.part" | shasum -a 256 -c --status; then
        echo "Checksum mismatch for $url" >&2
        rm -f "$file.part"
        exit 1
    fi
    mv "$file.part" "$file"
}

fetch "$KERNEL_URL" "$WORK/vmlinuz" "$KERNEL_SHA256"

# Virtualization.framework boots an uncompressed arm64 Image.
gunzip -c "$WORK/vmlinuz" > "$OUT/Image"
if [ "$(dd if="$OUT/Image" bs=1 skip=56 count=4 2>/dev/null)" != "ARMd" ]; then
    echo "$OUT/Image is not an uncompressed arm64 Linux Image" >&2
    exit 1
fi

echo "Kernel ready in $OUT/Image"
