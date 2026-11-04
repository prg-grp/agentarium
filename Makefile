# Development tasks. The guest kernel is downloaded into target/guest and the root
# filesystems are built into target/rootfs (workspace target dir).

APP := agent-toolbox
GUEST_DIR := target/guest
KERNEL := $(GUEST_DIR)/Image

.PHONY: guest rootfs run check clean-guest

guest: $(KERNEL) rootfs

$(KERNEL): scripts/build-guest.sh
	scripts/build-guest.sh

# The rootfs makefile decides itself which images are stale.
rootfs:
	$(MAKE) -C vms/rootfs

run: guest
	cargo run -p $(APP)

check:
	cargo fmt --all --check
	cargo check --workspace
	cargo clippy --workspace
	bash -n scripts/build-guest.sh
	sh -n scripts/codesign-run.sh

clean-guest:
	rm -rf $(GUEST_DIR)
	$(MAKE) -C vms/rootfs clean
