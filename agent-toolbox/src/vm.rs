//! The demo VM, run with Apple's Virtualization.framework.
//!
//! The framework is used on the main queue, so everything here must be called
//! on the main thread, which is Slint's UI thread on macOS. Completion handlers
//! also run there.

use std::os::fd::{IntoRawFd, OwnedFd};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSError, NSFileHandle, NSString, NSURL};
use objc2_virtualization::{
    VZDiskImageStorageDeviceAttachment, VZFileHandleNetworkDeviceAttachment,
    VZFileHandleSerialPortAttachment, VZLinuxBootLoader, VZVirtioBlockDeviceConfiguration,
    VZVirtioConsoleDeviceSerialPortConfiguration, VZVirtioEntropyDeviceConfiguration,
    VZVirtioNetworkDeviceConfiguration, VZVirtualMachine, VZVirtualMachineConfiguration,
    VZVirtualMachineState,
};

use crate::data::VmSpec;
use crate::net::{self, Network};
use crate::terminal::Console;

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
        for path in [spec.kernel, spec.rootfs] {
            if !Path::new(path).exists() {
                return Err(format!("{path} is missing. Run `make guest` first."));
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
                &file_url(spec.kernel),
            );
            // Linux guests get no usable hardware clock, so the guest sets its clock from this.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs();
            boot.setCommandLine(&NSString::from_str(&format!(
                "{} agent_toolbox.time={now}",
                spec.command_line
            )));

            let disk = VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(
                VZDiskImageStorageDeviceAttachment::alloc(),
                &file_url(spec.rootfs),
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

fn file_url(path: &str) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(path))
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
