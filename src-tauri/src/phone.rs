use std::{
    fs,
    future::Future,
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use idevice::{
    IdeviceError, IdeviceService,
    afc::{AfcClient, opcode::AfcFopenMode},
    lockdown::LockdownClient,
    pairing_file::PairingFile,
    provider::IdeviceProvider,
    usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdDevice, UsbmuxdListenEvent},
};
use tauri::AppHandle;
use tokio::{
    io::AsyncWriteExt,
    sync::mpsc::UnboundedSender,
    task::{AbortHandle, JoinSet},
};
use tokio_stream::StreamExt;

use crate::{
    Context, follow,
    library::{Library, room},
    said,
    state::{
        Batch, Device, Devices, End, Media, Need, Progress, Report, Settings, Trouble, check_name,
        local, render,
    },
    with,
};

/// What the camera roll holds: the camera's own files, and what was saved
/// or brought in from elsewhere (GIFs, screenshots, a camera's RAW files from
/// an SD card reader). The CLI took fewer and skipped the rest without a
/// word. Not .aae: an edit's recipe, which only Photos can read.
pub const MEDIA: [&str; 20] = [
    "heic", "heif", "jpg", "jpeg", "png", "dng", "mov", "mp4", "m4v", "gif", "tif", "tiff", "webp",
    "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2",
];

/// How long one exchange with the phone may take before we call it dead, and
/// how long a person gets to answer the trust prompt. Both are the CLI's
/// numbers, which were earned against real hardware.
const PATIENCE: Duration = Duration::from_secs(60);
const TRUST: Duration = Duration::from_secs(30);

/// The phone answers, or it does not answer at all. Without a deadline a
/// wedged AFC read hangs the scan or the copy for ever, and the only cure is
/// quitting the app.
async fn within<T, E: std::fmt::Display>(
    what: &str,
    call: impl Future<Output = Result<T, E>>,
) -> Result<T, String> {
    match tokio::time::timeout(PATIENCE, call).await {
        Ok(answered) => answered.doing(what),
        Err(_) => Err(format!("{what}: the iPhone stopped responding")),
    }
}

/// An AFC connection to one phone's filesystem, over usbmuxd.
pub async fn files_of(udid: &str) -> Result<AfcClient, String> {
    let addr = UsbmuxdAddr::from_env_var().doing("bad usbmuxd address")?;

    let mut muxer = within("usbmuxd is unreachable", addr.connect(0)).await?;

    // A phone that is also paired over Wi-Fi is attached twice under one udid,
    // and `get_device` answers with whichever of the two usbmuxd lists first.
    // The roster only ever offers the cable, so this has to ask for the cable:
    // a camera roll fetched over Wi-Fi is the same work at a fraction of the
    // speed, on a link that drops when the phone sleeps.
    let listed = within("the iPhone is no longer connected", muxer.get_devices()).await?;

    let device = listed
        .into_iter()
        .find(|device| device.udid == udid && device.connection_type == Connection::Usb)
        .ok_or_else(|| "the iPhone is no longer connected".to_string())?;

    let provider = device.to_provider(addr, "rollport");

    within("AFC refused the connection", AfcClient::connect(&provider)).await
}

/// The phone's own thumbnail of one camera-roll file: a JPEG iOS keeps ready,
/// about 80 KB. On an iPhone 12 Pro Max with iOS 27, 5703 of 5770 files had
/// one; the newest few may not yet, and a Live Photo's clip never does.
pub async fn thumbnail(udid: &str, path: &str) -> Result<Vec<u8>, String> {
    let mut afc = files_of(udid).await?;
    let place = format!("/PhotoData/Thumbnails/V2{path}/5005.JPG");

    let mut file = within("no thumbnail", afc.open(place, AfcFopenMode::RdOnly)).await?;
    let read = within("cannot read the thumbnail", file.read_entire()).await;

    // idevice asserts when an open file is dropped without being closed: a
    // panic in a dev build, a leaked descriptor in the one we ship.
    within("cannot close the thumbnail", file.close()).await?;

    read
}

/// Every photo and video in /DCIM, with what it weighs and when it was taken.
pub async fn walk(app: &AppHandle, udid: &str, ticket: u64) -> Result<Vec<Media>, String> {
    let mut afc = files_of(udid).await?;

    let mut pending = vec!["/DCIM".to_string()];
    let mut media = Vec::new();

    // A still, never a video: which .MOV is a Live Photo's clip, with no
    // thumbnail of its own, is only known once the walk ends.
    let mut latest = None;

    while let Some(directory) = pending.pop() {
        // Every folder under /DCIM has to be readable. A phone that locks
        // mid-walk fails the listing of whichever folder is next, and calling
        // that "an entry that turned out to be a file" hands back a camera roll
        // with a whole 10xAPPLE missing and nothing said about it.
        let names = within("cannot read the camera roll", afc.list_dir(&directory)).await?;

        for name in names {
            if name == "." || name == ".." {
                continue;
            }

            let path = format!("{directory}/{name}");

            // /DCIM holds folders like 100APPLE and files like IMG_0001.HEIC,
            // so the extension settles it for every file without a round trip
            // per entry. What has no extension is asked about — a dozen names
            // in a camera roll, and the alternative is guessing.
            let Some((_, extension)) = name.rsplit_once('.') else {
                let info = within("cannot read the camera roll", afc.get_file_info(&path)).await?;

                if info.st_ifmt == "S_IFDIR" {
                    pending.push(path)
                }

                continue;
            };

            if !MEDIA.contains(&extension.to_lowercase().as_str()) {
                continue;
            }

            let info = within("cannot read the camera roll", afc.get_file_info(&path)).await?;

            let file = Media {
                path,
                size: info.size as u64,
                mtime: info.modified,
                still: None,
            };

            if !file.is_video() {
                latest = Some(file.path.clone());
            }

            media.push(file);

            // Nobody waits for this walk any more: another phone was chosen,
            // or somebody looked again. Every question it asks slows the walk
            // that replaced it.
            if media.len() % 100 == 0
                && !with(app, |state| {
                    state.roll_seen(ticket, media.len(), latest.clone())
                })
            {
                return Err("nobody is waiting for this walk".into());
            }
        }
    }

    mark_live_videos(&mut media);

    Ok(media)
}

/// A .mov beside an image of the same name is the video half of a Live Photo,
/// and carries the image's time.
pub fn mark_live_videos(media: &mut [Media]) {
    let images: std::collections::HashMap<String, chrono::NaiveDateTime> = media
        .iter()
        .filter(|file| {
            matches!(
                extension(&file.path).as_deref(),
                Some("heic" | "heif" | "jpg" | "jpeg" | "dng")
            )
        })
        .map(|file| (stem(&file.path), file.mtime))
        .collect();

    for file in media.iter_mut() {
        file.still = match extension(&file.path).as_deref() {
            Some("mov") => images.get(&stem(&file.path)).copied(),
            _ => None,
        };
    }
}

pub fn extension(path: &str) -> Option<String> {
    path.rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
}

/// The whole path without its extension, so pairing is per folder.
pub fn stem(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(stem, _)| stem.to_lowercase())
        .unwrap_or_else(|| path.to_lowercase())
}

pub const CHUNK: usize = 1 << 20;

/// How often the window hears about progress.
const ANNOUNCE: Duration = Duration::from_millis(200);

/// How long it takes a change in speed to be most of what the rate says. A
/// long import should not be judged by how its first file went, and a phone
/// that slows down halfway should be believed within a few seconds.
const SETTLE: f64 = 8.0;

/// No time left is quoted before the rate has this many seconds behind it.
const WARM_UP: f64 = 3.0;

/// A quoted time left counts down with the clock, and is replaced only when it
/// has stood this long and the fresh estimate is this far from it. Quoted
/// again on every announcement, it changed several times a second.
const REQUOTE: f64 = 5.0;
const DRIFT: f64 = 0.1;

/// The time left to show, as (seconds left, quoted at), in seconds since the
/// run began: the standing quote, or a new one from `fresh` when there is
/// none yet or the standing one has drifted.
fn requote(quote: Option<(f64, f64)>, fresh: Option<f64>, now: f64) -> Option<(f64, f64)> {
    let Some(fresh) = fresh.filter(|_| now >= WARM_UP) else {
        return quote;
    };

    match quote {
        Some((left, at)) => {
            let shown = (left - (now - at)).max(0.0);
            let drifted = (fresh - shown).abs() > DRIFT * shown.max(1.0);

            if now - at >= REQUOTE && drifted {
                Some((fresh, now))
            } else {
                quote
            }
        }
        None => Some((fresh, now)),
    }
}

/// The file's own name, without the folders it sat in.
pub fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Four connections at once. When the files are small most of the time goes
/// on waiting for the phone to answer rather than on reading, so four fetches
/// overlap — the number the CLI has always asked of real hardware.
const WORKERS: usize = 4;

/// A file that fails gets two more goes, a moment apart. A wobble on one file
/// is not a reason to leave it for the next run, and a long run should not
/// stall on one that will never arrive.
const ATTEMPTS: u32 = 3;
const BACKOFF: Duration = Duration::from_millis(250);

pub async fn copy_all(
    app: &AppHandle,
    udid: &str,
    destination: &Path,
    media: Vec<Media>,
    settings: Settings,
    cancel: &Arc<AtomicBool>,
) -> Result<Report, String> {
    // That the phone will talk to us at all, established once, before four
    // workers each decide it for themselves, and before rollport-library.db is made: a
    // first run that cannot reach the phone writes nothing into the folder.
    let mut first = Some(files_of(udid).await?);

    // Opening rollport-library.db is blocking work, kept off the async workers.
    let opened = tokio::task::spawn_blocking({
        let destination = destination.to_path_buf();

        move || {
            // Before anything is written: another import into this folder, from
            // another window, user or computer, would fetch the same photos.
            let lock = lock_folder(&destination)?;

            // The one moment a rollport-library.db is brought into being, if it is not
            // there yet. Until now the folder was somewhere you had chosen,
            // not something we had written to. Files are named with the layout
            // it then holds: one that has imported keeps its own.
            let library = Library::create(&destination)?;
            library.write_settings(&settings)?;

            let layout = library
                .setting("layout")?
                .unwrap_or_else(crate::state::default_layout);

            sweep(&destination);

            Ok::<_, String>((library, layout, lock))
        }
    })
    .await
    .doing("cannot read rollport-library.db")?;

    // Held until the copy returns, and let go by the system if the app dies.
    let (library, layout, _lock) = opened?;
    let fetch = Arc::new(media);

    let total = fetch.len();
    let total_bytes = fetch.iter().map(|file| file.size).sum::<u64>();
    let started = Instant::now();
    let mut announced = Instant::now();

    // A rate that discounts the past, and the sample it is measured against.
    // Measured on bytes as they stream, not as files finish: four workers
    // finishing a big video in one sample read as a burst of speed.
    let streamed = Arc::new(AtomicU64::new(0));
    // What the files being copied right now have still to write: the disk
    // does not yet show it.
    let held = Arc::new(Mutex::new(0u64));
    let mut rate = 0f64;
    let mut sampled = Instant::now();
    let mut sampled_bytes = 0u64;
    let mut quote = None;

    let mut done = 0;
    let mut imported = 0;
    let mut lost = Lost::default();
    let mut bytes = 0;
    let mut copied_bytes = 0;
    let mut latest = None;
    let mut landed = Vec::new();

    // Why the run is over before its end, once the folder has said. The
    // workers are stopped by the flag, between chunks, and drained: aborting
    // one mid-read drops an AFC file unclosed, which idevice asserts against.
    let mut fatal = None;
    let mut full = false;

    let cursor = Arc::new(AtomicUsize::new(0));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut pool = tokio::task::JoinSet::new();

    for _ in 0..WORKERS {
        // The first worker inherits the connection that has already proved
        // itself. The rest open their own, and one that cannot open one leaves
        // its share of the queue to the others.
        let mut afc = first.take();

        let (fetch, cursor, tx, cancel, streamed, held) = (
            fetch.clone(),
            cursor.clone(),
            tx.clone(),
            cancel.clone(),
            streamed.clone(),
            held.clone(),
        );
        let (udid, destination, layout) =
            (udid.to_string(), destination.to_path_buf(), layout.clone());

        pool.spawn(async move {
            while !cancel.load(Ordering::Relaxed) {
                let index = cursor.fetch_add(1, Ordering::Relaxed);

                let Some(file) = fetch.get(index) else { break };

                // Only ever sent when the loop below breaks before its first
                // attempt, which is the one case: somebody pressed Stop.
                let mut outcome = Err(Failed::File("stopped".to_string()));

                for attempt in 0..ATTEMPTS {
                    // Stop is not a failure to retry. Without this the worker
                    // spends two more AFC handshakes and a second of backoff
                    // rediscovering that it was told to stop.
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }

                    let client = match &mut afc {
                        Some(client) => client,
                        None => match files_of(&udid).await {
                            Ok(open) => afc.insert(open),
                            Err(reason) => {
                                outcome = Err(Failed::File(reason));
                                tokio::time::sleep(BACKOFF).await;
                                continue;
                            }
                        },
                    };

                    outcome = copy(
                        client,
                        file,
                        &destination,
                        &layout,
                        &cancel,
                        &streamed,
                        &held,
                    )
                    .await;

                    if !worth_again(&outcome) {
                        break;
                    }

                    // It may have been the connection rather than the file, and
                    // a dead one would fail the next two attempts just as fast.
                    afc = None;

                    if attempt + 1 < ATTEMPTS {
                        tokio::time::sleep(BACKOFF).await;
                    }
                }

                if tx.send((index, outcome)).is_err() {
                    break;
                }
            }
        });
    }

    // Every worker holds a clone; this one would keep the loop below waiting
    // for a sender that never sends.
    drop(tx);

    while let Some((index, outcome)) = rx.recv().await {
        let file = &fetch[index];

        done += 1;
        bytes += file.size;

        let recorded = match outcome {
            Ok(Copied::Fetched) => {
                imported += 1;
                landed.push(file);
                copied_bytes += file.size;

                // A Live Photo's clip has no thumbnail; its still stands for it.
                if !file.is_live_video() {
                    latest = Some(file.path.clone());
                }

                library.record(file)
            }
            // It was already on disk but the folder had never been told.
            // Recording it is what stops it being offered as new for ever.
            Ok(Copied::Already) => library.record(file),
            // A folder that cannot be written to is the end of the run: every
            // file left would fail the same way, three times each, with a
            // timeout between.
            Err(Failed::Folder(reason)) => Err(reason),
            // What fits is copied; the rest waits for space. Not a failure, and
            // not the end: the files in flight finish, and a smaller one later
            // in the list may still fit. Said once, at the end.
            Err(Failed::Full) => {
                full = true;
                Ok(())
            }
            // A file we cannot read is one file, not the end of the run. The
            // rest of the camera roll is still worth copying, and the folder
            // will offer this one again next time.
            Err(Failed::File(reason)) => {
                lost.add(name_of(&file.path), reason, cancel.load(Ordering::Relaxed));
                Ok(())
            }
        };

        if let Err(reason) = recorded {
            cancel.store(true, Ordering::Relaxed);
            fatal.get_or_insert(reason);
        }

        // Often enough to look alive, rarely enough not to flood the window.
        if announced.elapsed() >= ANNOUNCE || done == total {
            announced = Instant::now();

            // Weighted by how long the sample took, so the smoothing does not
            // change meaning when the announcements bunch up or spread out.
            let over = sampled.elapsed().as_secs_f64();

            let arrived = streamed.load(Ordering::Relaxed);

            if over > 0.0 {
                let just_now = arrived.saturating_sub(sampled_bytes) as f64 / over;
                let weight = 1.0 - (-over / SETTLE).exp();

                rate = if rate == 0.0 {
                    just_now
                } else {
                    rate + (just_now - rate) * weight
                };

                sampled = Instant::now();
                sampled_bytes = arrived;
            }

            // What is still to fetch, over the speed it is being fetched at.
            let fresh = (rate > 1.0).then(|| total_bytes.saturating_sub(arrived) as f64 / rate);
            let now = started.elapsed().as_secs_f64();
            quote = requote(quote, fresh, now);

            with(app, |state| {
                state.progress(Progress {
                    done,
                    total,
                    bytes,
                    total_bytes,
                    eta: quote.map(|(left, at)| (left - (now - at)).max(0.0) as u64),
                    latest: latest.clone(),
                })
            });
        }
    }

    // A worker that panicked never reported its file. It is one that did not
    // arrive, not one that was never there.
    while let Some(joined) = pool.join_next().await {
        // After Stop, that is idevice's debug assert on a phone that went
        // away mid-close (see `Lost::add`).
        if joined.is_err() {
            lost.add(
                "",
                "the copy stopped unexpectedly".into(),
                cancel.load(Ordering::Relaxed),
            );
        }
    }

    let end = ending(fatal, full, cancel.load(Ordering::Relaxed));

    Ok(Report {
        imported,
        bytes: copied_bytes,
        seconds: started.elapsed().as_secs(),
        end,
        newest: Batch::of(landed).newest,
        udid: udid.to_string(),
        // One unreadable file is not a reason to abandon the other nine
        // thousand, so what it was is reported at the end of the run.
        trouble: lost.trouble(),
    })
}

/// The files that did not arrive: how many, and the first, by name, with what
/// it said. Which file, and what it said, is the difference between one bad
/// photo and a phone that locked halfway through.
#[derive(Default)]
struct Lost {
    failed: usize,
    first: Option<(String, String)>,
}

impl Lost {
    /// One file that did not arrive. After Stop it is not lost: the stop
    /// broke it off, the phone still has all of it, and the folder takes it
    /// next time. Nor after a broken folder, which raises the stop flag too
    /// and is said once, as the run's end.
    fn add(&mut self, name: &str, reason: String, stopped: bool) {
        if stopped {
            return;
        }

        self.failed += 1;
        self.first.get_or_insert_with(|| (name.to_string(), reason));
    }

    fn trouble(self) -> Option<Trouble> {
        let failed = self.failed;
        self.first.map(|(name, reason)| Trouble {
            failed,
            name,
            reason,
        })
    }
}

/// The file whose lock says an import is writing into the folder.
const FOLDER_LOCK: &str = ".rollport-lock";

/// Take the folder for this import: Err while another import holds it. The
/// lock is the system's, so it never outlives its holder. A folder that
/// cannot be locked at all — some network shares — is imported into without.
fn lock_folder(destination: &Path) -> Result<Option<fs::File>, String> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(destination.join(FOLDER_LOCK))
        .doing("cannot write into the folder")?;

    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(fs::TryLockError::WouldBlock) => {
            Err("Another import is writing into this folder. Import again once it is done.".into())
        }
        Err(fs::TryLockError::Error(_)) => Ok(None),
    }
}

/// How a run ended. A broken folder also raises the stop flag, and a disk
/// that filled up is worth saying over a Stop: either is the news.
fn ending(fatal: Option<String>, full: bool, stopped: bool) -> End {
    match fatal {
        Some(reason) => End::Failed { reason },
        None if full => End::Full,
        None if stopped => End::Stopped,
        None => End::Done,
    }
}

/// Whether a file is worth another attempt. Only one that failed by itself:
/// a folder or a disk that failed it would fail every other file the same way.
fn worth_again(outcome: &Result<Copied, Failed>) -> bool {
    matches!(outcome, Err(Failed::File(_)))
}

/// A size as a person reads it, for the one sentence Rust writes with one.
fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    // One decimal below 10 of a unit, as it rounds: 9.96 MB is "10 MB".
    let shown = |size: f64, unit: usize| {
        let places = usize::from(size < 9.95 && unit > 0);
        format!("{size:.places$}")
    };

    let mut size = n as f64;
    let mut unit = 0;

    // The next unit once the number would show as 1000: "1.0 MB", not
    // "1000 KB". The window says sizes the same way (`size` in flow.ts).
    while shown(size, unit)
        .parse::<f64>()
        .is_ok_and(|shown| shown >= 1000.0)
        && unit < UNITS.len() - 1
    {
        size /= 1000.0;
        unit += 1;
    }

    // A no-break space: the number and its unit never land on two lines.
    format!("{}\u{a0}{}", shown(size, unit), UNITS[unit])
}

/// The name a file is copied under until it is whole: where it lives on the
/// phone, which no other file shares, and this process, because another copy
/// of the app may be fetching the same photo into the same folder.
fn scratch_name(path: &str) -> String {
    format!(
        ".{}.{}.part",
        path.trim_start_matches('/').replace('/', "_"),
        std::process::id()
    )
}

/// How long a scratch file sits untouched before it belongs to nobody. A copy
/// writes to its file every fraction of a second, and a read that stalls
/// longer than PATIENCE fails and takes its file with it.
const ABANDONED: Duration = Duration::from_secs(10 * 60);

/// A run that was killed rather than stopped — the app quit, the machine went
/// down — leaves its scratch files behind, and one of them can be four
/// gigabytes. They are hidden, so nothing else will ever mention them. Only
/// the abandoned ones go: another window may be importing into this folder,
/// and a network drive may not honour a lock that would have kept it out.
fn sweep(destination: &Path) {
    let Ok(entries) = fs::read_dir(destination) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // Ours, and not a download of somebody else's. Asked first: with a
        // flat layout every photo in the library is in this listing.
        if !(name.starts_with(".DCIM_") && name.ends_with(".part")) {
            continue;
        }

        // Asked of the file: on Windows the listing's time can be stale.
        let abandoned = fs::metadata(entry.path())
            .and_then(|meta| meta.modified())
            .is_ok_and(|when| when.elapsed().is_ok_and(|age| age > ABANDONED));

        if abandoned {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// What became of one file.
pub enum Copied {
    /// Fetched from the phone.
    Fetched,
    /// Already on disk under the name this layout gives it, so nothing was
    /// fetched — but the folder had not recorded it, and now can.
    Already,
}

/// Why one file did not arrive, and whether the run can go on without it.
#[derive(Debug)]
pub enum Failed {
    /// This file, this time. Another go may work, and the rest of the camera
    /// roll is still worth copying.
    File(String),
    /// The folder or the disk it sits on. Every other file would fail the same
    /// way, so there is nothing to be had from trying nine thousand more.
    Folder(String),
    /// The file would leave the disk too full. The run stops here, and the
    /// rest waits for space.
    Full,
}

/// Anything the folder end of a copy says is the end of the run.
trait Local<T> {
    fn on_disk(self, what: &str) -> Result<T, Failed>;
}

impl<T> Local<T> for Result<T, std::io::Error> {
    fn on_disk(self, what: &str) -> Result<T, Failed> {
        self.map_err(|e| match e.kind() {
            // The disk filled while the file was written, by something else or
            // past what the room check saw: the same as not fitting at all.
            std::io::ErrorKind::StorageFull => Failed::Full,
            _ => Failed::Folder(format!("{what}: {e}")),
        })
    }
}

pub async fn copy(
    afc: &mut AfcClient,
    file: &Media,
    destination: &Path,
    layout: &str,
    cancel: &AtomicBool,
    streamed: &AtomicU64,
    held: &Mutex<u64>,
) -> Result<Copied, Failed> {
    let name = name_of(&file.path);
    let rendered = render(layout, name, local(file.taken()));

    // A layout is checked before a folder is allowed to keep it, but one may
    // have been written into rollport-library.db by hand or by an older run. Leaving the
    // file on the phone and saying why beats writing outside the folder.
    if check_name(&rendered).is_err() {
        return Err(Failed::Folder(
            "the naming pattern gives this file no name in the folder".into(),
        ));
    }

    let target = destination.join(rendered);

    // A layout with slashes in it means folders that may not exist yet.
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).on_disk("cannot make the folder")?;
    }

    // The name is derived from the file, so a file we copied before lands on
    // the name it already has. Size alone is not enough to say it is the same
    // file — two shots can share a name and a size under the {name} layout —
    // so the capture time has to match too, and we set that on every copy.
    // The legacy CLI never made this claim at all; it suffixed and downloaded
    // again. This is that, minus re-downloading what is demonstrably here.
    if let Ok(existing) = fs::metadata(&target)
        && existing.len() == file.size
        && existing
            .modified()
            .ok()
            .and_then(|when| when.duration_since(UNIX_EPOCH).ok())
            .is_some_and(|since| since.as_secs() as i64 == file.mtime.and_utc().timestamp())
    {
        return Ok(Copied::Already);
    }

    // Room for this file beside the ones still arriving, held until it is
    // whole or gone. While they arrive it waits for them: what they land is
    // what the room is, and they may be smaller than they promised. Alone, a
    // file that does not fit does not fit.
    let mut room_held = loop {
        match Held::take(held, room(destination), file.size) {
            Ok(room_held) => break room_held,
            Err(arriving) if arriving > 0 => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Failed::File("stopped".into()));
                }
                tokio::time::sleep(BACKOFF).await;
            }
            Err(_) => return Err(Failed::Full),
        }
    };

    // One file, one scratch name. Four workers can render one final name — the
    // {name} layout gives IMG_0001.HEIC to a shot in every folder on the phone —
    // and a name that was only asked about is still free when the bytes arrive.
    // So the scratch name is where the file lives on the phone, which nothing
    // else shares, and the real name is taken once there is a whole file for it.
    // It sits at the top of the folder, where the next run looks for leftovers,
    // whatever subfolder the layout puts the file in.
    let partial = destination.join(scratch_name(&file.path));

    // Nothing half-written survives a failure. `Partial` removes the file on
    // any early return, and is disarmed once the rename has happened.
    struct Partial<'a>(Option<&'a Path>);

    impl Drop for Partial<'_> {
        fn drop(&mut self) {
            if let Some(path) = self.0 {
                let _ = fs::remove_file(path);
            }
        }
    }

    let mut clean = Partial(Some(&partial));

    let mut handle = within(
        "cannot open the file on the iPhone",
        afc.open(&*file.path, AfcFopenMode::RdOnly),
    )
    .await
    .map_err(Failed::File)?;

    // Every way out of the read closes the handle. idevice asserts when a file
    // descriptor is dropped unclosed, so a `?` straight out of the loop is a
    // panicked worker in a dev build and a leaked descriptor in the one we
    // ship — and either way a file that is never reported, which is how a run
    // that ran out of disk finished saying nothing went wrong.
    let read: Result<u64, Failed> = async {
        let mut sink = tokio::fs::File::create(&partial)
            .await
            .on_disk("cannot write the file")?;
        loop {
            // Between files is too late to stop when the file is a four-gigabyte
            // video. What has arrived of it goes when `Partial` does.
            if cancel.load(Ordering::Relaxed) {
                return Err(Failed::File("stopped".into()));
            }

            let bytes = within("cannot read the file", handle.read_n(CHUNK))
                .await
                .map_err(Failed::File)?;

            if bytes.is_empty() {
                break;
            }

            sink.write_all(&bytes)
                .await
                .on_disk("cannot write the file")?;
            streamed.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            room_held.wrote(bytes.len() as u64);
        }

        // write_all only fills tokio's buffer: the last chunk's error arrives
        // here or nowhere, and on a full disk that chunk is the short one.
        sink.flush().await.on_disk("cannot write the file")?;

        // rollport-library.db says the photo is here once it is recorded, and that record
        // is synced. The photo has to be on the disk before the record is.
        sink.sync_data().await.on_disk("cannot write the file")?;

        // What the disk holds, not what the phone sent.
        Ok(sink
            .metadata()
            .await
            .on_disk("cannot write the file")?
            .len())
    }
    .await;

    let closed = within("cannot close the file", handle.close())
        .await
        .map_err(Failed::File);

    // What went wrong with the file first, and only then what went wrong with
    // letting go of it.
    let written = read?;
    closed?;

    // A short read or a short write is a truncated photo. Cheaper to catch
    // here than to leave a half file behind under a name that says it is whole.
    if written != file.size {
        return Err(Failed::File(format!(
            "only {} of {} arrived",
            bytes(written),
            bytes(file.size)
        )));
    }

    // Only a whole file gets the real name.
    let target = claim(target)?;

    if let Err(reason) = fs::rename(&partial, &target).on_disk("cannot rename the file") {
        // The name was reserved for a file that never made it.
        let _ = fs::remove_file(&target);
        return Err(reason);
    }

    clean.0 = None;

    // The photo is from when it was taken, not from when it was copied. Set on
    // the real name: a scratch file with an old date looks abandoned to the
    // sweep of another window.
    let _ = fs::File::options()
        .write(true)
        .open(&target)
        .and_then(|done| done.set_modified(SystemTime::from(file.mtime.and_utc())));

    Ok(Copied::Fetched)
}

/// One file's share of the room on the disk, from before its first byte
/// until it is whole or gone.
/// `held` is what the files arriving have still to write: what they have
/// written, the room on the disk already counts.
struct Held<'a> {
    held: &'a Mutex<u64>,
    left: u64,
}

impl<'a> Held<'a> {
    /// Err when the file does not fit beside the ones arriving, with what
    /// they have still to write: none, and it does not fit at all.
    fn take(held: &'a Mutex<u64>, room: u64, size: u64) -> Result<Self, u64> {
        let mut total = held.lock().unwrap_or_else(|e| e.into_inner());

        if total.saturating_add(size) > room {
            return Err(*total);
        }

        *total += size;
        Ok(Self { held, left: size })
    }

    /// These bytes are on the disk now, and in the room it reports.
    fn wrote(&mut self, bytes: u64) {
        let bytes = bytes.min(self.left);
        self.left -= bytes;
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) -= bytes;
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) -= self.left;
    }
}

/// A name nothing else has taken, suffixed _1, _2 and so on. Taken by making
/// the file, not by finding the name free: two workers asking the same question
/// a moment apart both hear yes, and the second one lands on the first one's
/// photo. Creating it is the question and the answer in one step.
pub fn claim(target: PathBuf) -> Result<PathBuf, Failed> {
    let stem = target
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let extension = target.extension().map(|e| e.to_string_lossy().into_owned());

    for n in 0.. {
        let candidate = match n {
            0 => target.clone(),
            n => {
                let mut name = format!("{stem}_{n}");

                if let Some(extension) = &extension {
                    name.push('.');
                    name.push_str(extension);
                }

                target.with_file_name(name)
            }
        };

        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Failed::Folder(format!("cannot write the file: {e}"))),
        }
    }

    unreachable!()
}

pub const RETRY: Duration = Duration::from_secs(1);

/// How long usbmuxd may list no phone while one is on the cable before it
/// counts as hung: udev starts it within about a second of the plug.
const SILENT: Duration = Duration::from_secs(5);

/// Whether the USB bus holds an iPhone, iPad or iPod, read from sysfs. Where
/// there is no sysfs, as on a Mac or Windows, never.
fn iphone_on_usb() -> bool {
    let Ok(devices) = fs::read_dir("/sys/bus/usb/devices") else {
        return false;
    };
    devices.flatten().any(|device| {
        let read = |name| fs::read_to_string(device.path().join(name)).unwrap_or_default();
        is_iphone(read("idVendor").trim(), read("idProduct").trim())
    })
}

/// The products usbmuxd's own udev rule starts it for: 05ac:12[9a]x and
/// 05ac:190[1-5]. A Mac's built-in keyboard is Apple too, and is not one.
fn is_iphone(vendor: &str, product: &str) -> bool {
    let product = product.to_ascii_lowercase();
    let bytes = product.as_bytes();
    vendor.eq_ignore_ascii_case("05ac")
        && bytes.len() == 4
        && bytes[3].is_ascii_hexdigit()
        && (product.starts_with("129")
            || product.starts_with("12a")
            || (product.starts_with("190") && (b'1'..=b'5').contains(&bytes[3])))
}

/// How often a phone that waits for trust is asked again. Each question is a
/// fresh pairing request, keys and all: every second is faster than a person
/// can reach for a phone, and in a debug build the keys alone cost more.
const ASK_AGAIN: Duration = Duration::from_secs(3);

pub fn watch(app: AppHandle) {
    loop {
        // A new runtime each time, so a panic leaves nothing of the last one
        // running: dropping it cancels every ask.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build the usbmuxd runtime");

        match std::panic::catch_unwind(AssertUnwindSafe(|| runtime.block_on(watch_once(&app)))) {
            Ok(Ok(())) => {}
            // On Linux, udev starts usbmuxd when an iPhone is plugged in and
            // stops it after the last one leaves: silence there means no phone.
            #[cfg(target_os = "linux")]
            Ok(Err(_)) if !iphone_on_usb() => heard(&app, Devices::Listed(Vec::new())),
            Ok(Err(reason)) => heard(&app, Devices::Unavailable { reason }),
            Err(panic) => heard(
                &app,
                Devices::Unavailable {
                    reason: format!("the iPhone watcher failed: {}", said(panic)),
                },
            ),
        }

        std::thread::sleep(RETRY);
    }
}

async fn watch_once(app: &AppHandle) -> Result<(), String> {
    // USBMUXD_SOCKET_ADDRESS moves every connection, as it does for
    // libimobiledevice's tools.
    let addr = UsbmuxdAddr::from_env_var().doing("bad usbmuxd address")?;
    let mut muxer = tokio::time::timeout(PATIENCE, addr.connect(0))
        .await
        .map_err(|_| "usbmuxd did not answer".to_string())?
        .doing("usbmuxd is unreachable")?;
    // usbmuxd replays an Attached for every device already plugged in, so this
    // reports the current state before it reports any change.
    let mut events = tokio::time::timeout(PATIENCE, muxer.listen())
        .await
        .map_err(|_| "usbmuxd did not answer".to_string())?
        .doing("usbmuxd is unreachable")?;
    heard(app, Devices::Listed(Vec::new()));

    // Each attached phone with its status once known. Asks run beside this
    // loop, so an unplug is heard while a phone is still being asked.
    let mut roster: Vec<(u32, Option<Device>, AbortHandle)> = Vec::new();
    let mut asks = JoinSet::new();
    let (answer, mut answers) = tokio::sync::mpsc::unbounded_channel();
    let mut check = tokio::time::interval(SILENT);
    let mut alone = false;

    loop {
        tokio::select! {
            // usbmuxd can hang and still take connections, listing nothing:
            // an iPhone on the cable and none listed, twice in a row, is that.
            _ = check.tick() => {
                let was = alone;
                alone = roster.is_empty() && iphone_on_usb();
                if was && alone {
                    heard(app, Devices::Unavailable { reason: "usbmuxd lists no iPhone, but one is plugged in".into() });
                }
                continue;
            }
            event = events.next() => match event {
                // A phone that is also paired over Wi-Fi is attached twice, once
                // per transport, under one udid. This app imports over the
                // cable, and the window would list the same phone twice.
                Some(Ok(UsbmuxdListenEvent::Connected(device)))
                    if device.connection_type != Connection::Usb => continue,
                Some(Ok(UsbmuxdListenEvent::Connected(device))) => {
                    let id = device.device_id;
                    roster.push((id, None, asks.spawn(ask(device, answer.clone()))));
                    continue;
                }
                Some(Ok(UsbmuxdListenEvent::Disconnected(id))) => roster.retain(|(attached, _, asking)| {
                    let stays = *attached != id;
                    if !stays {
                        asking.abort();
                    }
                    stays
                }),
                Some(Err(e)) => return Err(format!("usbmuxd is unreachable: {e}")),
                None => return Ok(()),
            },
            Some((id, status)) = answers.recv() => {
                // usbmuxd gives a new id on every attach, so an answer from a
                // phone that has left finds nothing here.
                let Some(entry) = roster.iter_mut().find(|(attached, ..)| *attached == id) else {
                    continue;
                };
                entry.1 = Some(status);
            }
        }

        // A JoinSet keeps each finished ask until it is joined.
        while asks.try_join_next().is_some() {}

        heard(
            app,
            Devices::Listed(
                roster
                    .iter()
                    .filter_map(|(_, status, _)| status.clone())
                    .collect(),
            ),
        );
    }
}

/// Land what usbmuxd said, and read the phone it made ready.
fn heard(app: &AppHandle, devices: Devices) {
    follow(app, with(app, |state| state.devices(devices)))
}

/// Ask one phone for its status, and again while a person can still answer it
/// on the phone: trust is granted there and usbmuxd says nothing about it.
async fn ask(device: UsbmuxdDevice, answer: UnboundedSender<(u32, Device)>) {
    loop {
        let status = status_of(&device).await;
        let again = asks_again(&status);

        if answer.send((device.device_id, status)).is_err() || !again {
            return;
        }

        tokio::time::sleep(ASK_AGAIN).await;
    }
}

/// A phone worth asking again. One that refused trust only asks again once it
/// is plugged in again, and every question is a fresh pairing request.
fn asks_again(status: &Device) -> bool {
    matches!(status, Device::Waiting { need, .. } if *need != Need::Replug)
}

async fn status_of(device: &UsbmuxdDevice) -> Device {
    match describe(device).await {
        Ok((name, version)) => Device::Ready {
            udid: device.udid.clone(),
            name,
            version,
        },
        Err(need) => Device::Waiting {
            udid: device.udid.clone(),
            need,
        },
    }
}

/// The device's name and iOS version, from one lockdown session.
pub async fn describe(device: &UsbmuxdDevice) -> Result<(String, String), Need> {
    let addr = UsbmuxdAddr::from_env_var().doing("bad usbmuxd address")?;
    let provider = device.to_provider(addr, "rollport");

    // Without a trusted pairing there is no pairing file. macOS writes one when
    // Finder meets the phone and the Apple service does it on Windows, but
    // nothing does on Linux, and nothing does anywhere once trust has been
    // reset. So when there is none we ask for it ourselves, which is the thing
    // that puts the Trust prompt on the phone.
    let pairing = match tokio::time::timeout(TRUST, provider.get_pairing_file()).await {
        Ok(Ok(pairing)) => pairing,
        _ => trust(device).await?,
    };

    let mut lockdown = within(
        "lockdown refused the connection",
        LockdownClient::connect(&provider),
    )
    .await?;

    // A record usbmuxd still holds but the phone has forgotten — trust reset on
    // the phone, or a record carried over from another machine. Lockdown says
    // InvalidHostID and will go on saying it every three seconds for ever, so
    // the record is asked for again, which is what puts the prompt back.
    let refused = match tokio::time::timeout(PATIENCE, lockdown.start_session(&pairing)).await {
        Ok(Ok(_)) => None,
        Ok(Err(refused)) => Some(refused),
        Err(_) => {
            return Err(Need::from(
                "lockdown refused the session: the iPhone stopped responding".to_string(),
            ));
        }
    };

    if let Some(refused) = refused {
        // Trusted, but locked: the same ask as a locked phone that has not
        // paired yet, not a failure.
        if matches!(refused, IdeviceError::PasswordProtected) {
            return Err(Need::Unlock);
        }
        if !matches!(refused, IdeviceError::InvalidHostID) {
            return Err(format!("lockdown refused the session: {refused}").into());
        }

        let pairing = trust(device).await?;

        lockdown = within(
            "lockdown refused the connection",
            LockdownClient::connect(&provider),
        )
        .await?;

        within(
            "lockdown refused the session",
            lockdown.start_session(&pairing),
        )
        .await?;
    }

    let name = value(&mut lockdown, "DeviceName").await?;
    let version = value(&mut lockdown, "ProductVersion").await?;

    Ok((name, version))
}

/// Ask the phone to trust this computer, and keep its answer.
///
/// The prompt this raises is answered by a person holding the phone, so this
/// asks once and reports what the phone said rather than standing there
/// waiting: the watcher comes back in a moment and asks again. What comes back
/// goes to usbmuxd, so the trust granted here is the trust every later run and
/// every other tool on this computer sees.
async fn trust(device: &UsbmuxdDevice) -> Result<PairingFile, Need> {
    let addr = UsbmuxdAddr::from_env_var().doing("bad usbmuxd address")?;
    let mut muxer = within("usbmuxd is unreachable", addr.connect(0)).await?;
    let provider = device.to_provider(addr, "rollport");

    // usbmuxd's own name for this computer. Using it as the host identity makes
    // the pairing the same on every run without the app having to remember
    // anything of its own.
    let buid = within("usbmuxd gave no host identity", muxer.get_buid()).await?;

    let mut lockdown = within(
        "lockdown refused the connection",
        LockdownClient::connect(&provider),
    )
    .await?;

    match tokio::time::timeout(TRUST, lockdown.pair_once(&buid, &buid, None)).await {
        Ok(Ok(pairing)) => {
            let record = pairing
                .clone()
                .serialize()
                .doing("cannot read the pairing back")?;

            within(
                "usbmuxd would not save the pairing",
                muxer.save_pair_record(&device.udid, record),
            )
            .await?;

            Ok(pairing)
        }
        Ok(Err(refused)) => Err(need_of(Some(refused))),
        Err(_) => Err(need_of(None)),
    }
}

/// What a phone that would not pair asks of the person: its refusal, or None
/// when it gave no answer in time.
fn need_of(refused: Option<IdeviceError>) -> Need {
    match refused {
        // The prompt is on the phone and nobody has answered it yet, or not
        // in time. The watcher asks again.
        Some(IdeviceError::PairingDialogResponsePending) | None => Need::Trust,
        // Refused: the phone asks again only after a replug.
        Some(IdeviceError::UserDeniedPairing) => Need::Replug,
        // A locked phone cannot show the prompt.
        Some(IdeviceError::PasswordProtected) => Need::Unlock,
        Some(e) => format!("the iPhone would not pair: {e}").into(),
    }
}

pub async fn value(lockdown: &mut LockdownClient, key: &str) -> Result<String, String> {
    within(
        &format!("lockdown gave no {key}"),
        lockdown.get_value(Some(key), None),
    )
    .await?
    .into_string()
    .ok_or_else(|| format!("{key} was not a string"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_products_usbmuxd_serves_count_as_an_iphone() {
        assert!(is_iphone("05ac", "12a8")); // iPhone
        assert!(is_iphone("05AC", "129A")); // older iPad, upper case
        assert!(is_iphone("05ac", "1905"));
        assert!(!is_iphone("05ac", "1906"));
        assert!(!is_iphone("05ac", "0262")); // a MacBook's own keyboard
        assert!(!is_iphone("05ac", "8600")); // T1 chip: usbmuxd, but no photos
        assert!(!is_iphone("1234", "12a8"));
        assert!(!is_iphone("05ac", "12a"));
    }

    #[test]
    fn a_disk_that_fills_while_writing_is_full_not_failed() {
        let full: Result<(), std::io::Error> =
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull));
        assert!(matches!(
            full.on_disk("cannot write the file"),
            Err(Failed::Full)
        ));

        let denied: Result<(), std::io::Error> =
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(matches!(
            denied.on_disk("cannot write the file"),
            Err(Failed::Folder(_))
        ));
    }

    #[test]
    fn a_file_holds_its_room_until_it_is_whole_or_gone() {
        let held = Mutex::new(0);

        // More than the disk can give, with nothing arriving: it does not fit.
        assert_eq!(Held::take(&held, 100, 101).err(), Some(0));
        assert_eq!(*held.lock().unwrap(), 0);

        // Two files that fit only one at a time: the second is told what the
        // first has still to write, and waits.
        let mut first = Held::take(&held, 100, 51).expect("fits alone");
        assert_eq!(Held::take(&held, 100, 51).err(), Some(51));

        // What it writes leaves the hold and shows in the disk's room instead,
        // so it is not counted twice.
        first.wrote(31);
        assert_eq!(*held.lock().unwrap(), 20);
        let second = Held::take(&held, 100 - 31, 49).expect("fits beside it");
        drop(first);
        assert_eq!(*held.lock().unwrap(), 49);
        drop(second);
        assert_eq!(*held.lock().unwrap(), 0);
    }

    #[test]
    fn a_size_is_said_as_it_rounds() {
        for (n, said) in [
            (0, "0 B"),
            (999, "999 B"),
            (1_000, "1.0 KB"),
            (9_940_000, "9.9 MB"),
            (9_960_000, "10 MB"),
            (999_499, "999 KB"),
            (999_950, "1.0 MB"),
            (999_600_000, "1.0 GB"),
        ] {
            assert_eq!(bytes(n), said.replace(' ', "\u{a0}"));
        }
    }

    #[test]
    fn a_file_is_lost_only_while_the_run_goes_on() {
        let mut lost = Lost::default();
        lost.add("IMG_0001.HEIC", "timed out".into(), false);
        lost.add("IMG_0002.HEIC", "gone".into(), false);
        lost.add("IMG_0003.HEIC", "stopped".into(), true);

        let trouble = lost.trouble().expect("two were lost");
        assert_eq!(trouble.failed, 2);
        assert_eq!(
            (trouble.name.as_str(), trouble.reason.as_str()),
            ("IMG_0001.HEIC", "timed out")
        );

        let mut stopped = Lost::default();
        stopped.add("a file", "the copy stopped unexpectedly".into(), true);
        assert!(stopped.trouble().is_none());
    }

    /// Learned on real phones; each answer is a different thing to tell the
    /// person, and testing one by hand means refusing Trust on a phone.
    #[test]
    fn a_refusal_to_pair_says_what_to_do() {
        assert!(matches!(need_of(None), Need::Trust));
        assert!(matches!(
            need_of(Some(IdeviceError::PairingDialogResponsePending)),
            Need::Trust
        ));
        assert!(matches!(
            need_of(Some(IdeviceError::UserDeniedPairing)),
            Need::Replug
        ));
        assert!(matches!(
            need_of(Some(IdeviceError::PasswordProtected)),
            Need::Unlock
        ));
        assert!(matches!(
            need_of(Some(IdeviceError::InvalidHostID)),
            Need::Failed { .. }
        ));
    }

    /// A second import into the folder waits for the first to let go.
    #[test]
    fn one_import_at_a_time_writes_into_a_folder() {
        let dir = std::env::temp_dir().join(format!("rollport-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let first = lock_folder(&dir).unwrap();
        assert!(first.is_some());
        assert!(lock_folder(&dir).is_err());

        drop(first);
        assert!(lock_folder(&dir).unwrap().is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_ends_with_its_news_first() {
        let folder = || Some("cannot write the file".to_string());

        for (fatal, full, stopped, end) in [
            (folder(), true, true, "failed"),
            (folder(), false, false, "failed"),
            (None, true, true, "full"),
            (None, false, true, "stopped"),
            (None, false, false, "done"),
        ] {
            let said = match ending(fatal, full, stopped) {
                End::Failed { .. } => "failed",
                End::Full => "full",
                End::Stopped => "stopped",
                End::Done => "done",
                End::Unplugged => "unplugged",
            };
            assert_eq!(said, end);
        }
    }

    #[test]
    fn only_a_file_that_failed_by_itself_is_tried_again() {
        assert!(worth_again(&Err(Failed::File("timed out".into()))));
        assert!(!worth_again(&Err(Failed::Folder("read-only".into()))));
        assert!(!worth_again(&Err(Failed::Full)));
        assert!(!worth_again(&Ok(Copied::Fetched)));
        assert!(!worth_again(&Ok(Copied::Already)));
    }

    #[test]
    fn time_left_waits_then_holds_then_follows_a_real_change() {
        // Nothing in the first seconds, however sure the rate looks.
        assert_eq!(requote(None, Some(100.0), 1.0), None);

        let first = requote(None, Some(100.0), 3.0);
        assert_eq!(first, Some((100.0, 3.0)));

        // Within the five seconds the quote stands, even far off.
        assert_eq!(requote(first, Some(300.0), 7.0), first);

        // Later, a fresh estimate close to the count-down keeps it.
        assert_eq!(requote(first, Some(93.0), 10.0), first);

        // A fresh one well away from it replaces it.
        assert_eq!(requote(first, Some(150.0), 10.0), Some((150.0, 10.0)));

        // No rate to judge by leaves the quote as it was.
        assert_eq!(requote(first, None, 20.0), first);
    }

    fn when(text: &str) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// Two workers, one rendered name, no filesystem to look at by hand.
    #[test]
    fn one_name_cannot_be_claimed_twice() {
        let dir = std::env::temp_dir().join(format!("rollport-claim-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let wanted = dir.join("IMG_0001.HEIC");

        assert_eq!(claim(wanted.clone()).unwrap(), wanted);
        assert_eq!(claim(wanted.clone()).unwrap(), dir.join("IMG_0001_1.HEIC"));
        assert_eq!(claim(wanted).unwrap(), dir.join("IMG_0001_2.HEIC"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// A name with no extension, or with dots in its stem, gets its number
    /// before the last dot only.
    #[test]
    fn a_second_name_keeps_its_dots_and_its_lack_of_extension() {
        let dir = std::env::temp_dir().join(format!("rollport-dots-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        for (wanted, second) in [("IMG", "IMG_1"), ("a.b.HEIC", "a.b_1.HEIC")] {
            claim(dir.join(wanted)).unwrap();
            assert_eq!(claim(dir.join(wanted)).unwrap(), dir.join(second));
        }

        let _ = fs::remove_dir_all(&dir);
    }

    /// A refusal is answered on the phone only after a replug: asking again
    /// would put a new Trust prompt on it every few seconds.
    #[test]
    fn a_phone_that_refused_trust_is_not_asked_again() {
        let waiting = |need| Device::Waiting {
            udid: "a".into(),
            need,
        };
        assert!(!asks_again(&waiting(Need::Replug)));
        assert!(asks_again(&waiting(Need::Trust)));
        assert!(asks_again(&waiting(Need::Unlock)));
        assert!(asks_again(&waiting(Need::Failed {
            reason: "lockdown".into()
        })));
        assert!(!asks_again(&Device::Ready {
            udid: "a".into(),
            name: "iPhone".into(),
            version: "26.1".into(),
        }));
    }

    #[test]
    fn the_video_half_of_a_live_photo_is_marked() {
        let file = |path: &str| Media {
            path: path.into(),
            size: 1,
            mtime: when("2026-09-02 15:04:11"),
            still: None,
        };

        let mut media = vec![
            file("/DCIM/100APPLE/IMG_0001.HEIC"),
            file("/DCIM/100APPLE/IMG_0001.MOV"),
            file("/DCIM/100APPLE/IMG_0002.MOV"),
            file("/DCIM/101APPLE/IMG_0001.MOV"),
            // A ProRAW Live Photo.
            file("/DCIM/102APPLE/IMG_0003.DNG"),
            file("/DCIM/102APPLE/IMG_0003.MOV"),
        ];

        mark_live_videos(&mut media);

        let marked: Vec<bool> = media.iter().map(|file| file.is_live_video()).collect();
        // Only the .mov sharing a name with an image in its own folder.
        assert_eq!(marked, vec![false, true, false, false, false, true]);
    }

    /// The clip is written a second or more after its still. Named with its
    /// own time, the pair would get two names, and other apps would show a
    /// photo and a separate video.
    #[test]
    fn a_live_photo_clip_is_named_and_dated_with_its_still() {
        let file = |path: &str, at: &str| Media {
            path: path.into(),
            size: 1,
            mtime: when(at),
            still: None,
        };

        let mut media = vec![
            file("/DCIM/100APPLE/IMG_0001.HEIC", "2026-09-02 23:59:59"),
            file("/DCIM/100APPLE/IMG_0001.MOV", "2026-09-03 00:00:02"),
            file("/DCIM/100APPLE/IMG_0002.MOV", "2026-09-03 00:00:02"),
        ];

        mark_live_videos(&mut media);

        let layout = "{mtime:%Y-%m-%d_%H-%M-%S}_{name}";
        let names: Vec<String> = media
            .iter()
            .map(|file| render(layout, name_of(&file.path), file.taken()))
            .collect();
        assert_eq!(
            names,
            [
                "2026-09-02_23-59-59_IMG_0001.HEIC",
                "2026-09-02_23-59-59_IMG_0001.MOV",
                // A film keeps its own time.
                "2026-09-03_00-00-02_IMG_0002.MOV",
            ]
        );

        // Its own time is still what says whether the file is on disk.
        assert_eq!(media[1].mtime, when("2026-09-03 00:00:02"));
    }

    /// A killed run's leftovers go. A copy another window is still writing
    /// stays, and so does a download of somebody else's.
    #[test]
    fn the_sweep_takes_only_our_abandoned_scratch_files() {
        let dir = std::env::temp_dir().join(format!("rollport-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let make = |name: &str, age: Duration| {
            let file = fs::File::create(dir.join(name)).unwrap();
            file.set_modified(SystemTime::now() - age).unwrap();
        };

        let dead = scratch_name("/DCIM/100APPLE/IMG_0001.HEIC");
        let live = scratch_name("/DCIM/100APPLE/IMG_0002.HEIC");
        make(&dead, Duration::from_secs(3600));
        make(&live, Duration::from_secs(5));
        make(".film.mp4.part", Duration::from_secs(3600));

        sweep(&dir);

        assert!(!dir.join(&dead).exists());
        assert!(dir.join(&live).exists());
        assert!(dir.join(".film.mp4.part").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
