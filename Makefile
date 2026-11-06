# Development tasks. The guest kernel is downloaded into target/guest and the base root
# filesystem is built into target/rootfs (workspace target dir). The app clones instance
# disks from the base rootfs named by AGENT_TOOLBOX_BASE_ROOTFS.

APP := app
GUEST_DIR := target/guest
KERNEL := $(GUEST_DIR)/Image
BASE_ROOTFS := $(abspath target/rootfs/base.ext4)
# The guest supervisor, a static Linux binary baked into the base rootfs.
GUEST_TARGET := aarch64-unknown-linux-musl
SUPERVISOR := $(abspath target/$(GUEST_TARGET)/release/terrarium-supervisor)
# The app's Application Support folder (database, instance disks); named by db::APP_ID.
APP_DATA := $(HOME)/Library/Application Support/com.programming-group.terrarium

.PHONY: guest rootfs run check clean-guest clean-app

guest: $(KERNEL) rootfs

$(KERNEL): scripts/build-guest.sh
	scripts/build-guest.sh

# Cargo and the rootfs makefile decide themselves whether the supervisor and the image are
# stale. Needs the target once: rustup target add aarch64-unknown-linux-musl
rootfs:
	cargo build --release -p guest_supervisor --target $(GUEST_TARGET)
	$(MAKE) -C vms/rootfs SUPERVISOR=$(SUPERVISOR)

run: guest
	AGENT_TOOLBOX_BASE_ROOTFS=$(BASE_ROOTFS) cargo run -p $(APP)

check:
	cargo fmt --all --check
	cargo check --workspace --exclude guest_supervisor
	cargo check -p guest_supervisor --target $(GUEST_TARGET)
	cargo clippy --workspace --exclude guest_supervisor
	cargo clippy -p guest_supervisor --target $(GUEST_TARGET)
	bash -n scripts/build-guest.sh
	bash -n vms/provision/claude-code.sh
	bash -n vms/provision/custom.sh
	sh -n scripts/codesign-run.sh

clean-guest:
	rm -rf $(GUEST_DIR)
	$(MAKE) -C vms/rootfs clean

# Deletes all app data: the database and every instance with its disk.
clean-app:
	rm -rf "$(APP_DATA)"
