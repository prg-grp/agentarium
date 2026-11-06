# Development tasks. The guest kernel is downloaded into target/guest and the base root
# filesystem is built into target/rootfs (workspace target dir). The app clones instance
# disks from the base rootfs named by AGENT_TOOLBOX_BASE_ROOTFS.

APP := app
GUEST_DIR := target/guest
KERNEL := $(GUEST_DIR)/Image
BASE_ROOTFS := $(abspath target/rootfs/base.ext4)
# The app's Application Support folder (database, instance disks); named by db::APP_ID.
APP_DATA := $(HOME)/Library/Application Support/com.programming-group.terrarium

.PHONY: guest rootfs run check clean-guest clean-app

guest: $(KERNEL) rootfs

$(KERNEL): scripts/build-guest.sh
	scripts/build-guest.sh

# The rootfs makefile decides itself whether the image is stale.
rootfs:
	$(MAKE) -C vms/rootfs

run: guest
	AGENT_TOOLBOX_BASE_ROOTFS=$(BASE_ROOTFS) cargo run -p $(APP)

check:
	cargo fmt --all --check
	cargo check --workspace
	cargo clippy --workspace
	bash -n scripts/build-guest.sh
	bash -n vms/provision/claude-code.sh
	bash -n vms/provision/custom.sh
	sh -n scripts/codesign-run.sh
	sh -n vms/rootfs/base/agent-toolbox-provision
	sh -n vms/rootfs/base/console-autologin

clean-guest:
	rm -rf $(GUEST_DIR)
	$(MAKE) -C vms/rootfs clean

# Deletes all app data: the database and every instance with its disk.
clean-app:
	rm -rf "$(APP_DATA)"
