# Builds every guest root filesystem into target/rootfs/<name>.ext4.
# Each <name>/Dockerfile becomes the image agent-toolbox/<name>. List new ones in ROOTFS,
# in build order; base comes first so the others can start FROM agent-toolbox/base.
ROOTFS := base claude
HEADROOM_MB := 1024
OUT := ../../target/rootfs
IMAGES := $(ROOTFS:%=$(OUT)/%.ext4)

SHELL := /bin/bash
.PHONY: all clean
.DELETE_ON_ERROR:
.SECONDEXPANSION:

all: $(IMAGES)

# Every other rootfs inherits from base, so a rebuilt base makes them stale.
$(filter-out $(OUT)/base.ext4,$(IMAGES)): $(OUT)/base.ext4

# Rebuilt when anything in its directory changes. The docker export is unpacked in an
# Alpine container whose mkfs.ext4 writes a raw ext4 image sized to the contents plus
# headroom; Virtualization.framework attaches it as a virtio-blk disk (/dev/vda).
# /.dockerenv is dropped: with it, systemd thinks it runs in a Docker container and skips
# kernel command line conditions, timesyncd and its clock floor.
$(OUT)/%.ext4: $$(shell find $$* -type f) Makefile
	docker build --platform linux/arm64 -t agent-toolbox/$* $*
	mkdir -p $(OUT)
	set -euo pipefail; \
	cid=$$(docker create --platform linux/arm64 agent-toolbox/$*); \
	trap 'docker rm $$cid >/dev/null' EXIT; \
	docker export $$cid | docker run --rm -i -v "$(abspath $(OUT))":/out alpine:3.22 sh -euc '\
	    apk add -q --no-cache e2fsprogs tar; \
	    mkdir /r; tar -xf - -C /r; rm -f /r/.dockerenv; \
	    kb=$$(( $$(du -sk /r | cut -f1) + $(HEADROOM_MB) * 1024 )); \
	    mkfs.ext4 -q -F -d /r /out/$*.ext4 $${kb}k'

clean:
	rm -rf $(OUT)
