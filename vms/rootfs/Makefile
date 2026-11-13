# Builds the base root filesystem into target/rootfs/base.ext4 from base/Dockerfile.
# It is harness-agnostic: every instance disk is an APFS clone of it, and the instance's
# provisioning script installs its agent on the first boot.
OUT := ../../target/rootfs

SHELL := /bin/bash
.PHONY: all clean
.DELETE_ON_ERROR:

all: $(OUT)/base.ext4

# Rebuilt when anything in base/ changes. The guest supervisor and observer aren't part of
# it: the app shares them with every boot (see base/Dockerfile). The docker export is unpacked
# in an Alpine container whose mkfs.ext4 writes a sparse raw ext4 image just bigger than
# its contents (a quarter and 512 MiB of headroom). The app extends each instance's clone
# to the instance's disk size before it boots, and the guest grows the filesystem to fill
# it (x-systemd.growfs in base/Dockerfile). 4K blocks and a 256 MiB journal are set
# explicitly: mke2fs picks smaller ones for small filesystems, and they don't change when
# it grows.
# Inode tables and journal are left unwritten (lazy init); the guest mounts root with
# noinit_itable, so they stay unwritten. Virtualization.framework attaches the image as a
# virtio-blk disk (/dev/vda).
# /.dockerenv is dropped: with it, systemd thinks it runs in a Docker container and skips
# kernel command line conditions.
$(OUT)/base.ext4: $(shell find base -type f) Makefile
	mkdir -p $(OUT)
	docker build --platform linux/arm64 -t agent-toolbox/base base
	set -euo pipefail; \
	cid=$$(docker create --platform linux/arm64 agent-toolbox/base); \
	trap 'docker rm $$cid >/dev/null' EXIT; \
	docker export $$cid | docker run --rm -i -v "$(abspath $(OUT))":/out alpine:3.22 sh -euc '\
	    apk add -q --no-cache e2fsprogs tar; \
	    mkdir /r; tar -xf - -C /r; rm -f /r/.dockerenv; \
	    size=$$(( $$(du -sm /r | cut -f1) * 5 / 4 + 512 )); \
	    mkfs.ext4 -q -F -b 4096 -J size=256 -E lazy_itable_init=1,lazy_journal_init=1 -d /r /out/base.ext4 $${size}M'

clean:
	rm -rf $(OUT)
