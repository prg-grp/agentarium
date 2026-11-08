# Builds the base root filesystem into target/rootfs/base.ext4 from base/Dockerfile.
# It is harness-agnostic: every instance disk is an APFS clone of it, and the instance's
# provisioning script installs its agent on the first boot.
SIZE := 64G
OUT := ../../target/rootfs

# The guest supervisor (agentarium-supervisor) and observer (agentarium-observer) binaries;
# the root Makefile builds and sets them.
SUPERVISOR ?=
OBSERVER ?=

SHELL := /bin/bash
.PHONY: all clean
.DELETE_ON_ERROR:

all: $(OUT)/base.ext4

# Rebuilt when anything in base/ or a guest binary changes. The docker export is unpacked
# in an Alpine container whose mkfs.ext4 writes a sparse raw ext4 image at its full virtual
# size, so guests never need a resize: the file only takes the space of the blocks written
# to it.
# Inode tables and journal are left unwritten (lazy init); the guest mounts root with
# noinit_itable, so they stay unwritten. Virtualization.framework attaches the image as a
# virtio-blk disk (/dev/vda).
# /.dockerenv is dropped: with it, systemd thinks it runs in a Docker container and skips
# kernel command line conditions, timesyncd and its clock floor.
# The binaries are handed to Docker as the named build contexts "supervisor" and "observer".
$(OUT)/base.ext4: $(shell find base -type f) Makefile $(SUPERVISOR) $(OBSERVER)
	@test -n "$(SUPERVISOR)" && test -n "$(OBSERVER)" || { echo 'SUPERVISOR or OBSERVER is not set; run `make rootfs` from the repository root.' >&2; exit 1; }
	mkdir -p $(OUT)/supervisor $(OUT)/observer
	cp $(SUPERVISOR) $(OUT)/supervisor/agentarium-supervisor
	cp $(OBSERVER) $(OUT)/observer/agentarium-observer
	docker build --platform linux/arm64 --build-context supervisor=$(OUT)/supervisor --build-context observer=$(OUT)/observer -t agent-toolbox/base base
	set -euo pipefail; \
	cid=$$(docker create --platform linux/arm64 agent-toolbox/base); \
	trap 'docker rm $$cid >/dev/null' EXIT; \
	docker export $$cid | docker run --rm -i -v "$(abspath $(OUT))":/out alpine:3.22 sh -euc '\
	    apk add -q --no-cache e2fsprogs tar; \
	    mkdir /r; tar -xf - -C /r; rm -f /r/.dockerenv; \
	    mkfs.ext4 -q -F -E lazy_itable_init=1,lazy_journal_init=1 -d /r /out/base.ext4 $(SIZE)'

clean:
	rm -rf $(OUT)
