//! An instance's VM, run with Apple's Virtualization.framework.
//!
//! The framework is used on the main queue, so everything here must be called
//! on the main thread, which is Slint's UI thread on macOS. Completion handlers
//! also run there.

use std::os::fd::{IntoRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSError, NSFileHandle, NSString, NSURL};
use objc2_virtualization::{
    VZDiskImageStorageDeviceAttachment, VZFileHandleNetworkDeviceAttachment,
    VZFileHandleSerialPortAttachment, VZLinuxBootLoader, VZSharedDirectory, VZSingleDirectoryShare,
    VZVirtioBlockDeviceConfiguration, VZVirtioConsoleDeviceSerialPortConfiguration,
    VZVirtioEntropyDeviceConfiguration, VZVirtioFileSystemDeviceConfiguration,
    VZVirtioNetworkDeviceConfiguration, VZVirtualMachine, VZVirtualMachineConfiguration,
    VZVirtualMachineState,
};

use crate::net::{self, Network};
use crate::terminal::Console;

/// The guest kernel, built into the workspace target directory by `make guest`.
const KERNEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/guest/Image");

/// Names the base root filesystem that instance disks are cloned from; `make run` sets it.
const BASE_ROOTFS_VAR: &str = "AGENT_TOOLBOX_BASE_ROOTFS";

/// The virtio-fs tag of the shared provisioning folder; the guest mounts it in provision mode.
const PROVISION_TAG: &str = "agent-toolbox-provision";

/// What the guest does after booting; it reads the mode from a kernel flag.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BootMode {
    /// The console logs in `ubuntu`, whose login starts the agent.
    Agent,
    /// The console logs in root without the agent.
    Maintenance,
    /// The guest runs the provisioning script from the shared folder, then powers off.
    Provision,
}

/// What to boot: an instance's resources and disk, in one of the boot modes.
pub struct VmSpec {
    pub cpus: usize,
    pub memory_mib: u64,
    /// Raw ext4 image, attached read-write as the guest's /dev/vda.
    pub rootfs: PathBuf,
    pub mode: BootMode,
    /// Host folder shared read-write with the guest as `PROVISION_TAG`. Provision mode
    /// reads `provision.sh` from it and writes the script's exit code to `status`.
    pub share: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VmState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
    Other,
}

pub struct Vm {
    machine: Retained<VZVirtualMachine>,
    /// The VM's network stack; dropped after the machine.
    _network: Network,
}

impl Vm {
    /// Configures the VM and returns it with the host end of its console.
    pub fn create(spec: &VmSpec) -> Result<(Vm, Console), String> {
        if !unsafe { VZVirtualMachine::isSupported() } {
            return Err("Virtualization isn't available on this Mac.".into());
        }
        for path in [Path::new(KERNEL), &spec.rootfs] {
            if !path.exists() {
                return Err(format!(
                    "{} is missing. Run `make guest` first.",
                    path.display()
                ));
            }
        }

        // The guest reads what the host writes, and the other way round.
        let (guest_reads, host_writes) = std::io::pipe().map_err(|e| e.to_string())?;
        let (host_reads, guest_writes) = std::io::pipe().map_err(|e| e.to_string())?;
        let console =
            Console::new(host_reads.into(), host_writes.into()).map_err(|e| e.to_string())?;
        let (network, guest_network) = Network::start().map_err(|e| e.to_string())?;

        unsafe {
            let boot = VZLinuxBootLoader::initWithKernelURL(
                VZLinuxBootLoader::alloc(),
                &file_url(Path::new(KERNEL)),
            );
            // Linux guests get no usable hardware clock, so the guest sets its clock from this.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs();
            // The guest reads the boot mode from this flag.
            let mode = match spec.mode {
                BootMode::Agent => "",
                BootMode::Maintenance => " agent_toolbox.mode=maintenance",
                BootMode::Provision => " agent_toolbox.mode=provision",
            };
            // noinit_itable: the base rootfs is sparse with unwritten inode tables, and
            // zeroing them in the background would allocate their space on the Mac.
            boot.setCommandLine(&NSString::from_str(&format!(
                "console=hvc0 root=/dev/vda rootflags=noinit_itable rw{mode} agent_toolbox.time={now}"
            )));

            let disk = VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(
                VZDiskImageStorageDeviceAttachment::alloc(),
                &file_url(&spec.rootfs),
                false,
            )
            .map_err(|error| error.localizedDescription().to_string())?;
            let rootfs = VZVirtioBlockDeviceConfiguration::initWithAttachment(
                VZVirtioBlockDeviceConfiguration::alloc(),
                &disk,
            );

            let attachment =
                VZFileHandleSerialPortAttachment::initWithFileHandleForReading_fileHandleForWriting(
                    VZFileHandleSerialPortAttachment::alloc(),
                    Some(&file_handle(guest_reads.into())),
                    Some(&file_handle(guest_writes.into())),
                );
            let serial = VZVirtioConsoleDeviceSerialPortConfiguration::new();
            serial.setAttachment(Some(&attachment));

            let network_attachment = VZFileHandleNetworkDeviceAttachment::initWithFileHandle(
                VZFileHandleNetworkDeviceAttachment::alloc(),
                &file_handle(guest_network.into()),
            );
            network_attachment.setMaximumTransmissionUnit(net::MTU as isize);
            let nic = VZVirtioNetworkDeviceConfiguration::new();
            nic.setAttachment(Some(&network_attachment));

            let config = VZVirtualMachineConfiguration::new();
            config.setBootLoader(Some(&boot));
            config.setCPUCount(spec.cpus.clamp(
                VZVirtualMachineConfiguration::minimumAllowedCPUCount(),
                VZVirtualMachineConfiguration::maximumAllowedCPUCount(),
            ));
            config.setMemorySize((spec.memory_mib * 1024 * 1024).clamp(
                VZVirtualMachineConfiguration::minimumAllowedMemorySize(),
                VZVirtualMachineConfiguration::maximumAllowedMemorySize(),
            ));
            config.setSerialPorts(&NSArray::from_retained_slice(&[Retained::into_super(
                serial,
            )]));
            config.setStorageDevices(&NSArray::from_retained_slice(&[Retained::into_super(
                rootfs,
            )]));
            config.setNetworkDevices(&NSArray::from_retained_slice(&[Retained::into_super(nic)]));
            config.setEntropyDevices(&NSArray::from_retained_slice(&[Retained::into_super(
                VZVirtioEntropyDeviceConfiguration::new(),
            )]));
            if let Some(share) = &spec.share {
                let directory = VZSharedDirectory::initWithURL_readOnly(
                    VZSharedDirectory::alloc(),
                    &file_url(share),
                    false,
                );
                let single = VZSingleDirectoryShare::initWithDirectory(
                    VZSingleDirectoryShare::alloc(),
                    &directory,
                );
                let device = VZVirtioFileSystemDeviceConfiguration::initWithTag(
                    VZVirtioFileSystemDeviceConfiguration::alloc(),
                    &NSString::from_str(PROVISION_TAG),
                );
                device.setShare(Some(&single));
                config.setDirectorySharingDevices(&NSArray::from_retained_slice(&[
                    Retained::into_super(device),
                ]));
            }
            config
                .validateWithError()
                .map_err(|error| error.localizedDescription().to_string())?;

            let machine =
                VZVirtualMachine::initWithConfiguration(VZVirtualMachine::alloc(), &config);
            Ok((
                Vm {
                    machine,
                    _network: network,
                },
                console,
            ))
        }
    }

    pub fn start(&self, done: impl Fn(Result<(), String>) + 'static) {
        let handler = RcBlock::new(move |error: *mut NSError| done(result(error)));
        unsafe { self.machine.startWithCompletionHandler(&handler) };
    }

    /// Presses the VM's power button, so the guest shuts down cleanly.
    pub fn request_stop(&self) -> Result<(), String> {
        unsafe {
            if !self.machine.canRequestStop() {
                return Err("The virtual machine can't be asked to stop.".into());
            }
            self.machine
                .requestStopWithError()
                .map_err(|error| error.localizedDescription().to_string())
        }
    }

    /// Stops the VM immediately, like pulling the power cord. The fallback
    /// when the guest doesn't shut down after `request_stop`.
    pub fn stop(&self, done: impl Fn(Result<(), String>) + 'static) {
        let handler = RcBlock::new(move |error: *mut NSError| done(result(error)));
        unsafe { self.machine.stopWithCompletionHandler(&handler) };
    }

    pub fn state(&self) -> VmState {
        match unsafe { self.machine.state() } {
            VZVirtualMachineState::Stopped => VmState::Stopped,
            VZVirtualMachineState::Starting => VmState::Starting,
            VZVirtualMachineState::Running => VmState::Running,
            VZVirtualMachineState::Stopping => VmState::Stopping,
            VZVirtualMachineState::Error => VmState::Error,
            _ => VmState::Other,
        }
    }
}

/// Creates an instance's root disk at `disk` as an APFS clone of the base rootfs: instant,
/// and it shares the base's blocks until the guest writes. There is no copy fallback, as
/// a copy would write out the whole sparse image.
pub fn create_disk(disk: &Path) -> Result<(), String> {
    let base = std::env::var_os(BASE_ROOTFS_VAR)
        .map(PathBuf::from)
        .filter(|base| base.exists())
        .ok_or_else(|| {
            format!("{BASE_ROOTFS_VAR} names no base rootfs. Start the app with `make run`.")
        })?;
    if let Some(dir) = disk.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let c_path = |path: &Path| {
        std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())
    };
    let (from, to) = (c_path(&base)?, c_path(disk)?);
    if unsafe { libc::clonefile(from.as_ptr(), to.as_ptr(), 0) } != 0 {
        return Err(format!(
            "Couldn't clone {} to {}: {}. Both must be on the same APFS volume.",
            base.display(),
            disk.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn file_handle(fd: OwnedFd) -> Retained<NSFileHandle> {
    NSFileHandle::initWithFileDescriptor_closeOnDealloc(
        NSFileHandle::alloc(),
        fd.into_raw_fd(),
        true,
    )
}

fn result(error: *mut NSError) -> Result<(), String> {
    match unsafe { error.as_ref() } {
        Some(error) => Err(error.localizedDescription().to_string()),
        None => Ok(()),
    }
}
