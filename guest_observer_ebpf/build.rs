use which::which;

// Building the programs needs bpf-linker, which cargo doesn't know about. Rebuild when the
// bpf-linker on PATH changes.
fn main() {
    let bpf_linker = which("bpf-linker").expect("bpf-linker isn't installed; see README.md");
    println!("cargo:rerun-if-changed={}", bpf_linker.display());
}
