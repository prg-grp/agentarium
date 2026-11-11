//! An instance's VM, run with Apple's Virtualization.framework.
//!
//! Each VM operates on its own serial dispatch queue, so VMs run independently of each
//! other and of the UI thread. `Vm`'s methods can be called from any thread; they run
//! on that queue, where completion handlers also run.

use std::fs::File;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSFileHandle, NSString, NSURL};
use objc2_virtualization::{
    VZDiskImageStorageDeviceAttachment, VZFileHandleNetworkDeviceAttachment,
    VZFileHandleSerialPortAttachment, VZLinuxBootLoader, VZMultipleDirectoryShare,
    VZSharedDirectory, VZVirtioBlockDeviceConfiguration,
    VZVirtioConsoleDeviceSerialPortConfiguration, VZVirtioEntropyDeviceConfiguration,
    VZVirtioFileSystemDeviceConfiguration, VZVirtioNetworkDeviceConfiguration,
    VZVirtioSocketConnection, VZVirtioSocketDevice, VZVirtioSocketDeviceConfiguration,
    VZVirtualMachine, VZVirtualMachineConfiguration, VZVirtualMachineState,
};

use crate::net::{self, Network};
use crate::netlog::NetLog;
use crate::policy::Policy;
use crate::terminal::Console;

/// What the guest does after booting. The app sends it to the guest supervisor over the
/// command plane (`Start`); nothing runs on the session console before that.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BootMode {
    /// The supervisor runs the instance's launch command as `ubuntu`.
    Agent,
    /// The supervisor runs a root shell, without the agent.
    Maintenance,
    /// The supervisor runs the provisioning script, reports its exit code, then powers off.
    Provision,
}

/// What to boot: an instance's resources and disk.
pub struct VmSpec {
    pub cpus: usize,
    pub memory_mib: u64,
    /// The uncompressed arm64 Linux kernel the rootfs was made for (`guest::kernel_path`).
    pub kernel: PathBuf,
    /// Raw ext4 image, attached read-write as the guest's /dev/vda.
    pub rootfs: PathBuf,
    /// Host folders shared live through one virtio-fs device (`SHARE_TAG`); the guest
    /// supervisor mounts them where `Start.mounts` says.
    pub shares: Vec<SharedFolder>,
    /// What the guest may reach over the network.
    pub network: Arc<Policy>,
    /// Where the network's lookups and connections are recorded (agent boots).
    pub netlog: Option<NetLog>,
}

/// A host folder in the instance's virtio-fs share.
pub struct SharedFolder {
    /// Its name inside the share, as `Mount.share` names it to the guest.
    pub name: String,
    pub host_path: PathBuf,
    /// Enforced by the Mac, so nothing in the guest can write to it.
    pub read_only: bool,
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
    /// The serial queue the machine operates on.
    queue: DispatchRetained<DispatchQueue>,
    machine: Retained<VZVirtualMachine>,
    /// The VM's network stack; dropped after the machine.
    _network: Network,
}

// SAFETY: the machine is only used on its own queue (`self.queue`), which serializes
// every call to it, so `Vm` can be shared with and moved to other threads.
unsafe impl Send for Vm {}
unsafe impl Sync for Vm {}

impl Vm {
    /// Configures the VM and returns it with the host ends of its two consoles: the
    /// session (guest `hvc1`, the Terminal tab), then the boot log (guest `hvc0`, the
    /// kernel console, shown read-only in the Logs tab).
    pub fn create(spec: &VmSpec) -> Result<(Vm, Console, Console), String> {
        if !unsafe { VZVirtualMachine::isSupported() } {
            return Err("Virtualization isn't available on this Mac.".into());
        }
        for path in [&spec.kernel, &spec.rootfs] {
            if !path.exists() {
                return Err(format!("{} is missing.", path.display()));
            }
        }

        // A serial port and the host end of its console. The guest reads what the host
        // writes, and the other way round.
        let serial_port = || -> Result<_, String> {
            let (guest_reads, host_writes) = std::io::pipe().map_err(|e| e.to_string())?;
            let (host_reads, guest_writes) = std::io::pipe().map_err(|e| e.to_string())?;
            let console =
                Console::new(host_reads.into(), host_writes.into()).map_err(|e| e.to_string())?;
            let port = unsafe {
                let attachment =
                    VZFileHandleSerialPortAttachment::initWithFileHandleForReading_fileHandleForWriting(
                        VZFileHandleSerialPortAttachment::alloc(),
                        Some(&file_handle(guest_reads.into())),
                        Some(&file_handle(guest_writes.into())),
                    );
                let port = VZVirtioConsoleDeviceSerialPortConfiguration::new();
                port.setAttachment(Some(&attachment));
                port
            };
            Ok((console, port))
        };
        // The guest numbers the ports in this order: the log is hvc0, the session hvc1.
        let (log, log_port) = serial_port()?;
        let (session, session_port) = serial_port()?;
        let (network, guest_network) =
            Network::start(spec.network.clone(), spec.netlog.clone()).map_err(|e| e.to_string())?;

        unsafe {
            let boot = VZLinuxBootLoader::initWithKernelURL(
                VZLinuxBootLoader::alloc(),
                &file_url(&spec.kernel),
            );
            // Linux guests get no usable hardware clock, so the guest sets its clock from this.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs();
            // noinit_itable: the base rootfs is sparse with unwritten inode tables, and
            // zeroing them in the background would allocate their space on the Mac.
            boot.setCommandLine(&NSString::from_str(&format!(
                "console=hvc0 root=/dev/vda rootflags=noinit_itable rw agent_toolbox.time={now}"
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
            config.setSerialPorts(&NSArray::from_retained_slice(&[
                Retained::into_super(log_port),
                Retained::into_super(session_port),
            ]));
            config.setStorageDevices(&NSArray::from_retained_slice(&[Retained::into_super(
                rootfs,
            )]));
            config.setNetworkDevices(&NSArray::from_retained_slice(&[Retained::into_super(nic)]));
            config.setEntropyDevices(&NSArray::from_retained_slice(&[Retained::into_super(
                VZVirtioEntropyDeviceConfiguration::new(),
            )]));
            // The command plane to the guest supervisor (see `connect`).
            config.setSocketDevices(&NSArray::from_retained_slice(&[Retained::into_super(
                VZVirtioSocketDeviceConfiguration::new(),
            )]));
            if !spec.shares.is_empty() {
                let names: Vec<_> = spec
                    .shares
                    .iter()
                    .map(|share| NSString::from_str(&share.name))
                    .collect();
                let directories: Vec<_> = spec
                    .shares
                    .iter()
                    .map(|share| {
                        VZSharedDirectory::initWithURL_readOnly(
                            VZSharedDirectory::alloc(),
                            &file_url(&share.host_path),
                            share.read_only,
                        )
                    })
                    .collect();
                let names: Vec<&NSString> = names.iter().map(|name| &**name).collect();
                let share = VZMultipleDirectoryShare::initWithDirectories(
                    VZMultipleDirectoryShare::alloc(),
                    &NSDictionary::from_retained_objects(&names, &directories),
                );
                let device = VZVirtioFileSystemDeviceConfiguration::initWithTag(
                    VZVirtioFileSystemDeviceConfiguration::alloc(),
                    &NSString::from_str(command_plane::SHARE_TAG),
                );
                device.setShare(Some(&share));
                config.setDirectorySharingDevices(&NSArray::from_retained_slice(&[
                    Retained::into_super(device),
                ]));
            }
            config
                .validateWithError()
                .map_err(|error| error.localizedDescription().to_string())?;

            let queue = DispatchQueue::new("agent-toolbox.vm", DispatchQueueAttr::SERIAL);
            let machine = VZVirtualMachine::initWithConfiguration_queue(
                VZVirtualMachine::alloc(),
                &config,
                &queue,
            );
            Ok((
                Vm {
                    queue,
                    machine,
                    _network: network,
                },
                session,
                log,
            ))
        }
    }

    /// Starts the VM; `done` runs on the VM's queue once it has started or failed to.
    pub fn start(&self, done: impl Fn(Result<(), String>) + Send + 'static) {
        self.queue.exec_sync(|| {
            let handler = RcBlock::new(move |error: *mut NSError| done(result(error)));
            unsafe { self.machine.startWithCompletionHandler(&handler) };
        });
    }

    /// Presses the VM's power button, so the guest shuts down cleanly.
    pub fn request_stop(&self) -> Result<(), String> {
        let mut requested = Ok(());
        self.queue.exec_sync(|| unsafe {
            requested = if self.machine.canRequestStop() {
                self.machine
                    .requestStopWithError()
                    .map_err(|error| error.localizedDescription().to_string())
            } else {
                Err("The virtual machine can't be asked to stop.".into())
            };
        });
        requested
    }

    /// Stops the VM immediately, like pulling the power cord. The fallback when the
    /// guest doesn't shut down after `request_stop`. `done` runs on the VM's queue.
    pub fn stop(&self, done: impl Fn(Result<(), String>) + Send + 'static) {
        self.queue.exec_sync(|| {
            let handler = RcBlock::new(move |error: *mut NSError| done(result(error)));
            unsafe { self.machine.stopWithCompletionHandler(&handler) };
        });
    }

    /// Connects to the guest's vsock `port`; `done` runs on the VM's queue with the
    /// connection, or an error while nothing listens there yet.
    pub fn connect(&self, port: u32, done: impl Fn(Result<File, String>) + Send + 'static) {
        self.queue.exec_sync(|| {
            let handler = RcBlock::new(
                move |connection: *mut VZVirtioSocketConnection, error: *mut NSError| {
                    done(match unsafe { connection.as_ref() } {
                        // The connection closes its descriptor when it is released after
                        // this handler, so keep a duplicate.
                        Some(connection) => {
                            match unsafe { libc::dup(connection.fileDescriptor()) } {
                                fd if fd >= 0 => Ok(unsafe { File::from_raw_fd(fd) }),
                                _ => Err(std::io::Error::last_os_error().to_string()),
                            }
                        }
                        None => result(error).and(Err("no connection".into())),
                    })
                },
            );
            let device = unsafe { self.machine.socketDevices() }
                .firstObject()
                .and_then(|device| device.downcast::<VZVirtioSocketDevice>().ok());
            match device {
                Some(device) => unsafe { device.connectToPort_completionHandler(port, &handler) },
                None => handler.call((std::ptr::null_mut(), std::ptr::null_mut())),
            }
        });
    }

    pub fn state(&self) -> VmState {
        let mut state = VmState::Other;
        self.queue.exec_sync(|| {
            state = match unsafe { self.machine.state() } {
                VZVirtualMachineState::Stopped => VmState::Stopped,
                VZVirtualMachineState::Starting => VmState::Starting,
                VZVirtualMachineState::Running => VmState::Running,
                VZVirtualMachineState::Stopping => VmState::Stopping,
                VZVirtualMachineState::Error => VmState::Error,
                _ => VmState::Other,
            };
        });
        state
    }
}

/// Creates an instance's root disk at `disk` as an APFS clone of the base rootfs
/// (`guest::base_rootfs`): instant, and it shares the base's blocks until the guest writes.
/// There is no copy fallback, as a copy would write out the whole sparse image.
pub fn create_disk(base: &Path, disk: &Path) -> Result<(), String> {
    if let Some(dir) = disk.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let c_path = |path: &Path| {
        std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())
    };
    let (from, to) = (c_path(base)?, c_path(disk)?);
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
