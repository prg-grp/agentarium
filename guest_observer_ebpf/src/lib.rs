#![no_std]

// Only the binary (src/main.rs) is the eBPF program. This empty library lets guest_observer
// list the crate as a build dependency, so cargo rebuilds it when the programs change.
