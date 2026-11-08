// Compiles the eBPF programs (guest_observer_ebpf) for the BPF target with aya-build, which
// needs a nightly toolchain with rust-src and bpf-linker (see README.md). src/main.rs embeds
// the object from OUT_DIR.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let metadata = cargo_metadata::MetadataCommand::new().no_deps().exec()?;
    let package = metadata
        .packages
        .iter()
        .find(|package| package.name.as_str() == "guest_observer_ebpf")
        .ok_or("guest_observer_ebpf isn't in the workspace")?;
    let root_dir = package
        .manifest_path
        .parent()
        .ok_or("guest_observer_ebpf has no directory")?;
    aya_build::build_ebpf(
        [aya_build::Package {
            name: package.name.as_str(),
            root_dir: root_dir.as_str(),
            ..Default::default()
        }],
        aya_build::Toolchain::default(),
    )?;
    Ok(())
}
