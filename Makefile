# Development tasks. The guest kernel is downloaded into target/guest and the base root
# filesystem is built into target/rootfs (workspace target dir). The app clones instance
# disks from the base rootfs named by AGENT_TOOLBOX_BASE_ROOTFS.

APP := app
GUEST_DIR := target/guest
KERNEL := $(GUEST_DIR)/Image
BASE_ROOTFS := $(abspath target/rootfs/base.ext4)
# The guest supervisor and observer, static Linux binaries baked into the base rootfs.
GUEST_TARGET := aarch64-unknown-linux-musl
SUPERVISOR := $(abspath target/$(GUEST_TARGET)/release/agentarium-supervisor)
OBSERVER := $(abspath target/$(GUEST_TARGET)/release/agentarium-observer)
# Linux-only crates: checked for the guest target. guest_observer_ebpf is compiled for the
# BPF target by guest_observer's build script (nightly with rust-src, and bpf-linker).
GUEST_CRATES := -p guest_supervisor -p guest_observer
# Kernel struct bindings for the eBPF programs, generated from the kernel's BTF (needs Docker).
VMLINUX := guest_observer_ebpf/src/vmlinux.rs
# The third-party licenses the app shows in Preferences, generated from about.hbs.
# Needs cargo-about once: cargo install cargo-about
LICENSES := app/src/licenses.rs
HOST_ONLY := --exclude guest_supervisor --exclude guest_observer --exclude guest_observer_ebpf
# The app's Application Support folder (database, instance disks); named by db::APP_ID.
APP_DATA := $(HOME)/Library/Application Support/com.programming-group.agentarium

.PHONY: guest rootfs run check clean-guest clean-app

guest: $(KERNEL) rootfs

$(KERNEL): scripts/build-guest.sh
	scripts/build-guest.sh

$(VMLINUX): $(KERNEL) scripts/gen-vmlinux.sh
	scripts/gen-vmlinux.sh

$(LICENSES): Cargo.lock about.hbs about.toml
	cargo about generate --output-file $@ about.hbs

# Cargo and the rootfs makefile decide themselves whether the guest binaries and the image
# are stale. Needs the target once: rustup target add aarch64-unknown-linux-musl
rootfs: $(VMLINUX)
	cargo build --release $(GUEST_CRATES) --target $(GUEST_TARGET)
	$(MAKE) -C vms/rootfs SUPERVISOR=$(SUPERVISOR) OBSERVER=$(OBSERVER)

run: guest $(LICENSES)
	AGENT_TOOLBOX_BASE_ROOTFS=$(BASE_ROOTFS) cargo run -p $(APP)

check: $(VMLINUX) $(LICENSES)
	cargo fmt --all --check
	cargo check --workspace $(HOST_ONLY)
	cargo check $(GUEST_CRATES) --target $(GUEST_TARGET)
	cargo clippy --workspace $(HOST_ONLY)
	cargo clippy $(GUEST_CRATES) --target $(GUEST_TARGET)
	bash -n scripts/build-guest.sh
	bash -n scripts/gen-vmlinux.sh
	bash -n vms/provision/claude-code.sh
	bash -n vms/provision/custom.sh
	bash -n vms/provision/codex.sh
	bash -n vms/provision/antigravity.sh
	bash -n vms/provision/copilot.sh
	bash -n vms/provision/opencode.sh
	bash -n vms/provision/openclaw.sh
	bash -n vms/provision/hermes.sh
	bash -n vms/provision/agent-canvas.sh
	sh -n scripts/codesign-run.sh

clean-guest:
	rm -rf $(GUEST_DIR) $(VMLINUX)
	$(MAKE) -C vms/rootfs clean

# Deletes all app data: the database and every instance with its disk.
clean-app:
	rm -rf "$(APP_DATA)"
