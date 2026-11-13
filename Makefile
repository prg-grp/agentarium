# Development tasks. The guest kernel is downloaded into target/guest, the guest binaries
# are built into target/guest/bin and the base root filesystem is built into target/rootfs
# (workspace target dir). The app clones instance disks from the base rootfs named by
# AGENT_TOOLBOX_BASE_ROOTFS and shares target/guest/bin read-only with every boot.

APP := app
GUEST_DIR := target/guest
KERNEL := $(GUEST_DIR)/Image
BASE_ROOTFS := $(abspath target/rootfs/base.ext4)
# The guest supervisor and observer, static Linux binaries the app shares with the guest.
# They aren't part of the base rootfs, so changing them needs no new rootfs or instance.
GUEST_TARGET := aarch64-unknown-linux-musl
GUEST_BIN := $(GUEST_DIR)/bin
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

.PHONY: guest guest-bin rootfs run check clean-guest clean-app

guest: $(KERNEL) rootfs guest-bin

$(KERNEL): scripts/build-guest.sh
	scripts/build-guest.sh

$(VMLINUX): $(KERNEL) scripts/gen-vmlinux.sh
	scripts/gen-vmlinux.sh

$(LICENSES): Cargo.lock about.hbs about.toml
	cargo about generate --output-file $@ about.hbs

# Cargo decides itself whether the guest binaries are stale. install replaces each file
# with a new one, so a running guest keeps the binary it started. Needs the target once:
# rustup target add aarch64-unknown-linux-musl
guest-bin: $(VMLINUX)
	cargo build --release $(GUEST_CRATES) --target $(GUEST_TARGET)
	mkdir -p $(GUEST_BIN)
	install -m 755 target/$(GUEST_TARGET)/release/agentarium-supervisor $(GUEST_BIN)/
	install -m 755 target/$(GUEST_TARGET)/release/agentarium-observer $(GUEST_BIN)/

# The rootfs makefile decides itself whether the image is stale.
rootfs:
	$(MAKE) -C vms/rootfs

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
