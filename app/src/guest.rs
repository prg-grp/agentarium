//! The guest kernel and the base root filesystem that instance disks are cloned from.
//!
//! Release builds carry their GitHub release's tag and the SHA-256 of the compressed guest
//! assets attached to it (set by .github/workflows/release.yml). At launch, whatever is
//! missing is downloaded into Application Support, on the same APFS volume as the instance
//! disks, so `vm::create_disk` can clone the base. A disk's kernel modules match only the
//! kernel it was cloned for, so each instance records its kernel, and a kernel stays as
//! long as an instance uses it. Development builds use what `make guest` built in target/.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use sha2::{Digest, Sha256};
use slint::ComponentHandle;

use crate::db::{self, Db};
use crate::{AppWindow, GuestImage, GuestState};

/// Where releases, updates and guest assets are published.
pub const REPO_URL: &str = "https://github.com/prg-grp/agentarium";

/// The kernel of development builds: the one `make guest` downloads.
const DEV_KERNEL_ID: &str = "dev";
const DEV_KERNEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/guest/Image");
/// Names the base rootfs of development builds; `make run` sets it.
const BASE_ROOTFS_VAR: &str = "AGENT_TOOLBOX_BASE_ROOTFS";

/// Disk images are written in blocks of this size; all-zero blocks become holes.
const BLOCK: usize = 64 * 1024;

/// This build's release: where its guest assets are and what they hash to.
#[derive(Clone, Copy)]
struct Release {
    tag: &'static str,
    kernel: &'static str,
    rootfs: &'static str,
}

fn release() -> Option<Release> {
    let baked = |value: Option<&'static str>| value.filter(|value| !value.is_empty());
    Some(Release {
        tag: baked(option_env!("AGENTARIUM_RELEASE_TAG"))?,
        kernel: baked(option_env!("AGENTARIUM_KERNEL_SHA256"))?,
        rootfs: baked(option_env!("AGENTARIUM_ROOTFS_SHA256"))?,
    })
}

fn guest_dir() -> db::Result<PathBuf> {
    Ok(db::app_dir()?.join("guest"))
}

fn kernel_name(kernel: &str) -> String {
    format!("kernel-{kernel}")
}

fn rootfs_name(rootfs: &str) -> String {
    format!("rootfs-{rootfs}.ext4")
}

/// The kernel that new instances are created for.
pub fn current_kernel() -> &'static str {
    release().map_or(DEV_KERNEL_ID, |release| release.kernel)
}

/// The kernel image of instances created for `kernel`.
pub fn kernel_path(kernel: &str) -> db::Result<PathBuf> {
    if kernel == DEV_KERNEL_ID {
        return Ok(PathBuf::from(DEV_KERNEL));
    }
    Ok(guest_dir()?.join(kernel_name(kernel)))
}

/// The base rootfs that new instances are cloned from.
pub fn base_rootfs() -> Result<PathBuf, String> {
    let Some(release) = release() else {
        return std::env::var_os(BASE_ROOTFS_VAR)
            .map(PathBuf::from)
            .filter(|base| base.exists())
            .ok_or_else(|| {
                format!("{BASE_ROOTFS_VAR} names no base rootfs. Start the app with `make run`.")
            });
    };
    let base = guest_dir()
        .map_err(|error| error.to_string())?
        .join(rootfs_name(release.rootfs));
    match base.exists() && kernel_path(release.kernel).is_ok_and(|kernel| kernel.exists()) {
        true => Ok(base),
        false => Err("the guest image hasn't finished downloading.".into()),
    }
}

/// One file of this release that isn't here yet.
struct Download {
    step: &'static str,
    file: &'static str,
    sha256: &'static str,
    to: PathBuf,
}

enum Event {
    /// Bytes received of the current file, and its size if known.
    Progress {
        step: &'static str,
        received: u64,
        total: u64,
    },
    Done(Result<(), String>),
}

thread_local! {
    static ASSETS: RefCell<Weak<GuestAssets>> = const { RefCell::new(Weak::new()) };
}

/// Keeps this build's guest assets present and shows the download in `GuestImage`.
pub struct GuestAssets {
    ui: slint::Weak<AppWindow>,
    db: Rc<Db>,
    downloading: Cell<bool>,
}

impl GuestAssets {
    pub fn new(ui: &AppWindow, db: Rc<Db>) -> Rc<GuestAssets> {
        let assets = Rc::new(GuestAssets {
            ui: ui.as_weak(),
            db,
            downloading: Cell::new(false),
        });
        ASSETS.set(Rc::downgrade(&assets));
        let g = ui.global::<GuestImage>();
        g.set_source(match release() {
            Some(release) => format!("From release {}", release.tag).into(),
            None => "Built locally with make guest".into(),
        });
        let weak = Rc::downgrade(&assets);
        g.on_retry(move || {
            if let Some(assets) = weak.upgrade() {
                assets.start();
            }
        });
        assets.start();
        assets
    }

    /// Downloads whatever of this release's guest assets is missing, on a thread of its own.
    fn start(&self) {
        let Some(release) = release() else {
            return self.show(GuestState::Ready, "", "");
        };
        if self.downloading.get() {
            return;
        }
        let dir = match guest_dir().and_then(|dir| Ok(std::fs::create_dir_all(&dir).map(|()| dir)?))
        {
            Ok(dir) => dir,
            Err(error) => return self.show(GuestState::Failed, "", &error.to_string()),
        };
        let downloads: Vec<Download> = [
            Download {
                step: "Downloading kernel",
                file: "agentarium-kernel.zst",
                sha256: release.kernel,
                to: dir.join(kernel_name(release.kernel)),
            },
            Download {
                step: "Downloading root filesystem",
                file: "agentarium-rootfs.ext4.zst",
                sha256: release.rootfs,
                to: dir.join(rootfs_name(release.rootfs)),
            },
        ]
        .into_iter()
        .filter(|download| !download.to.exists())
        .collect();
        if downloads.is_empty() {
            self.show(GuestState::Ready, "", "");
            return self.remove_unused();
        }

        self.downloading.set(true);
        self.show(GuestState::Downloading, downloads[0].step, "");
        let report = |event: Event| {
            let _ = slint::invoke_from_event_loop(move || {
                ASSETS.with_borrow(|assets| {
                    if let Some(assets) = assets.upgrade() {
                        assets.handle(event);
                    }
                })
            });
        };
        let spawned = std::thread::Builder::new()
            .name("guest-download".into())
            .spawn(move || {
                let result = downloads.iter().try_for_each(|download| {
                    fetch(release.tag, download, &|received, total| {
                        report(Event::Progress {
                            step: download.step,
                            received,
                            total,
                        })
                    })
                });
                report(Event::Done(result));
            });
        if let Err(error) = spawned {
            self.downloading.set(false);
            self.show(GuestState::Failed, "", &error.to_string());
        }
    }

    fn handle(&self, event: Event) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let g = ui.global::<GuestImage>();
        match event {
            Event::Progress {
                step,
                received,
                total,
            } => {
                g.set_step(step.into());
                let mb = |bytes: u64| bytes.div_ceil(1_000_000);
                if total > 0 {
                    g.set_amount(format!("{} of {} MB", mb(received), mb(total)).into());
                    g.set_progress(received as f32 / total as f32);
                } else {
                    g.set_amount(format!("{} MB", mb(received)).into());
                }
            }
            Event::Done(result) => {
                self.downloading.set(false);
                match result {
                    Ok(()) => {
                        self.show(GuestState::Ready, "", "");
                        self.remove_unused();
                    }
                    Err(error) => self.show(GuestState::Failed, "", &error),
                }
            }
        }
    }

    fn show(&self, state: GuestState, step: &str, error: &str) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let g = ui.global::<GuestImage>();
        g.set_state(state);
        g.set_step(step.into());
        g.set_amount("".into());
        g.set_progress(0.0);
        g.set_error(error.into());
    }

    /// Deletes earlier releases' assets: base images, kernels no instance uses anymore,
    /// and unfinished downloads.
    fn remove_unused(&self) {
        let (Some(release), Ok(dir)) = (release(), guest_dir()) else {
            return;
        };
        let instances = match self.db.instances() {
            Ok(instances) => instances,
            Err(error) => return eprintln!("Couldn't read the instances' kernels: {error}"),
        };
        let keep: HashSet<String> = instances
            .iter()
            .map(|instance| kernel_name(&instance.kernel))
            .chain([kernel_name(release.kernel), rootfs_name(release.rootfs)])
            .collect();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            if !keep.contains(entry.file_name().to_string_lossy().as_ref())
                && let Err(error) = std::fs::remove_file(entry.path())
            {
                eprintln!("Couldn't remove {}: {error}", entry.path().display());
            }
        }
    }
}

/// Downloads one compressed asset of the release, unpacks it next to its destination and
/// moves it there once the download matches its hash.
fn fetch(tag: &str, download: &Download, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    let url = format!("{REPO_URL}/releases/download/{tag}/{}", download.file);
    let failed =
        |error: &dyn std::fmt::Display| format!("Couldn't download {}: {error}", download.file);
    let response = ureq::get(&url).call().map_err(|e| failed(&e))?;
    let total = response.body().content_length().unwrap_or(0);
    let mut source = Hashing {
        inner: response.into_body().into_reader(),
        hasher: Sha256::new(),
        received: 0,
        reported: 0,
        total,
        progress,
    };
    let partial = download.to.with_extension("partial");
    let unpacked = unpack(&mut source, &partial).and_then(|()| {
        // zstd stops at the end of its frame; whatever follows counts for the hash too.
        io::copy(&mut source, &mut io::sink()).map(|_| ())
    });
    if let Err(error) = unpacked {
        let _ = std::fs::remove_file(&partial);
        return Err(failed(&error));
    }
    let hash: String = source
        .hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if hash != download.sha256 {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("{} arrived damaged. Try again.", download.file));
    }
    std::fs::rename(&partial, &download.to).map_err(|e| failed(&e))
}

/// Decompresses a zstd stream into `to`, leaving all-zero blocks as holes so a sparse disk
/// image stays sparse.
fn unpack(from: impl Read, to: &Path) -> io::Result<()> {
    let mut decoder = zstd::stream::read::Decoder::new(from)?;
    // The release workflow compresses with --long=27.
    decoder.window_log_max(27)?;
    let mut file = File::create(to)?;
    let mut block = vec![0u8; BLOCK];
    let mut length = 0u64;
    loop {
        let mut filled = 0;
        while filled < BLOCK {
            match decoder.read(&mut block[filled..]) {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        if filled == 0 {
            break;
        }
        if block[..filled].iter().all(|&byte| byte == 0) {
            file.seek(SeekFrom::Current(filled as i64))?;
        } else {
            file.write_all(&block[..filled])?;
        }
        length += filled as u64;
        if filled < BLOCK {
            break;
        }
    }
    file.set_len(length)?;
    file.sync_all()
}

/// Hashes and counts what is read through it, reporting progress about every megabyte.
struct Hashing<'a, R> {
    inner: R,
    hasher: Sha256,
    received: u64,
    reported: u64,
    total: u64,
    progress: &'a dyn Fn(u64, u64),
}

impl<R: Read> Read for Hashing<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        self.received += read as u64;
        if self.received - self.reported >= 1_000_000
            || (read == 0 && self.received > self.reported)
        {
            self.reported = self.received;
            (self.progress)(self.received, self.total);
        }
        Ok(read)
    }
}
