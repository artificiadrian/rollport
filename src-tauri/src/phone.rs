use std::{
    collections::{HashMap, HashSet},
    fs,
    future::Future,
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use idevice::{
    IdeviceError, IdeviceService,
    afc::{AfcClient, errors::AfcError, opcode::AfcFopenMode},
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
        Batch, Device, Devices, End, Kind, MUTATIONS, Media, Need, Progress, Report, Settings,
        Trouble, check_name, here, render,
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
/// Photos' own record is read over a second connection while the walk runs:
/// the walk waits on round trips, the copy of the database on bandwidth.
/// Also why the files are undated, when Photos' record could not be read.
pub async fn walk(
    app: &AppHandle,
    udid: &str,
    ticket: u64,
) -> Result<(Vec<Media>, Option<String>), String> {
    // A walk that failed or was replaced has no use for the dates, and the
    // database is hundreds of megabytes on the same cable.
    let failed = AtomicBool::new(false);
    let wanted = || !failed.load(Ordering::Relaxed) && crate::lock(app).reading(ticket).is_some();

    let (walked, known) = tokio::join!(
        async {
            let walked = walk_dcim(app, udid, ticket).await;
            failed.store(walked.is_err(), Ordering::Relaxed);
            walked
        },
        photos(udid, ticket, &wanted)
    );
    let mut media = walked?;

    let mut edited = HashSet::new();
    let mut kept = Vec::new();
    let undated = match known {
        Ok(known) => {
            settle(&mut media, &known);
            kept = known
                .iter()
                .filter_map(|(path, known)| match known {
                    Known::Kept { taken, kind, name } => Some(Media {
                        path: path.clone(),
                        name: name.clone(),
                        size: 0,
                        mtime: chrono::NaiveDateTime::default(),
                        taken: *taken,
                        kind: kind.unwrap_or(Kind::of(name)),
                    }),
                    _ => None,
                })
                .collect();
            edited = media
                .iter()
                .filter(|file| {
                    matches!(
                        known.get(&file.path),
                        Some(Known::Taken { edited: true, .. })
                    )
                })
                .map(|file| file.path.clone())
                .collect();
            None
        }
        Err(reason) => {
            crate::note(
                app,
                &format!("dating files by when they were written: {reason}"),
            );
            Some(reason)
        }
    };

    mark_live_videos(&mut media);

    // Edits and iCloud's originals are files of their own outside /DCIM.
    // Without them the import still stands, and a later one takes them.
    if !edited.is_empty() || !kept.is_empty() {
        match beside(udid, &media, &edited, kept, &wanted).await {
            Ok(found) => media.extend(found),
            Err(reason) => crate::note(
                app,
                &format!("cannot read the edits or iCloud's originals: {reason}"),
            ),
        }
    }

    Ok((media, undated))
}

/// What `call` answers, or None when the file or folder is not on the phone.
/// Any other error, such as a locked phone, is an error.
async fn if_there<T>(
    what: &str,
    call: impl Future<Output = Result<T, IdeviceError>>,
) -> Result<Option<T>, String> {
    within(what, async {
        match call.await {
            Err(IdeviceError::Afc(AfcError::ObjectNotFound)) => Ok(None),
            answered => answered.map(Some),
        }
    })
    .await
}

/// Photos' render of each edited file, and the originals iCloud keeps on the
/// phone, as far as they are there yet: Photos lists an edit before it is
/// rendered, and an original before its file has arrived.
async fn beside(
    udid: &str,
    media: &[Media],
    edited: &HashSet<String>,
    kept: Vec<Media>,
    wanted: &impl Fn() -> bool,
) -> Result<Vec<Media>, String> {
    let mut afc = files_of(udid).await?;
    let mut found = Vec::new();

    for original in media.iter().filter(|file| edited.contains(&file.path)) {
        if !wanted() {
            return Ok(Vec::new());
        }
        found.extend(edits(&mut afc, original).await?);
    }

    for file in kept {
        if !wanted() {
            return Ok(Vec::new());
        }
        let Some(info) = if_there("cannot read an original", afc.get_file_info(&file.path)).await?
        else {
            continue;
        };
        // Its path, size and mtime are what the folder knows it by.
        found.push(Media {
            size: info.size as u64,
            mtime: info.modified,
            ..file
        });
    }

    Ok(found)
}

/// Photos' render of one edited file: FullSizeRender.jpg, .heic or .mov,
/// beside the recipe of the edit. It is dated and sorted with its original;
/// the clip of an edited Live Photo is that Live Photo's clip.
async fn edits(afc: &mut AfcClient, original: &Media) -> Result<Vec<Media>, String> {
    let Some((folder, name)) = original.path.rsplit_once('/') else {
        return Ok(Vec::new());
    };
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let adjustments = format!("{MUTATIONS}{folder}/{stem}/Adjustments");

    let Some(names) = if_there("cannot read an edit", afc.list_dir(&adjustments)).await? else {
        return Ok(Vec::new());
    };

    let mut found = Vec::new();
    for name in names
        .iter()
        .filter(|name| name.starts_with("FullSizeRender."))
    {
        let path = format!("{adjustments}/{name}");
        let Some(info) = if_there("cannot read an edit", afc.get_file_info(&path)).await? else {
            continue;
        };
        let kind = match (original.kind, Kind::of(&path)) {
            (Kind::LivePhoto, Kind::Video) => Kind::LiveVideo,
            (kind, _) => kind,
        };

        found.push(Media {
            name: edited_name(&original.name, &path),
            path,
            size: info.size as u64,
            mtime: info.modified,
            taken: original.taken,
            kind,
        });
    }

    Ok(found)
}

async fn walk_dcim(app: &AppHandle, udid: &str, ticket: u64) -> Result<Vec<Media>, String> {
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
                kind: Kind::of(&path),
                name: name.clone(),
                path,
                size: info.size as u64,
                mtime: info.modified,
                taken: here(info.modified),
            };

            if file.kind != Kind::Video {
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

    Ok(media)
}

/// What the Photos app knows about one file in /DCIM.
#[derive(Debug, PartialEq)]
pub enum Known {
    Taken {
        taken: chrono::DateTime<chrono::FixedOffset>,
        /// What it is, when Photos says more than the file's name does.
        kind: Option<Kind>,
        /// Edited in Photos: its render sits under /PhotoData/Mutations.
        edited: bool,
    },
    /// An original iCloud Photos keeps on the phone outside /DCIM: taken on
    /// another device, or shared to this one. Photos names its file by an
    /// id, so it carries the name it had where it was taken.
    Kept {
        taken: chrono::DateTime<chrono::FixedOffset>,
        kind: Option<Kind>,
        name: String,
    },
    /// Photos does not show it: it is in Recently Deleted, where it stays in
    /// /DCIM for 30 days, or it is a frame of a burst that was not picked.
    Hidden,
}

/// Core Data counts seconds from 2001-01-01 UTC.
const APPLE_EPOCH: i64 = 978_307_200;

/// What Photos knows about each file in /DCIM, by its path on the phone.
async fn photos(
    udid: &str,
    ticket: u64,
    wanted: &(dyn Fn() -> bool + Sync),
) -> Result<HashMap<String, Known>, String> {
    let scratch =
        std::env::temp_dir().join(format!("rollport-photos-{}-{ticket}", std::process::id()));

    sweep_photos();

    let read = async {
        fs::create_dir_all(&scratch).doing("cannot make a scratch folder")?;
        let mut afc = files_of(udid).await?;

        let (db, wal) = (
            scratch.join("Photos.sqlite"),
            scratch.join("Photos.sqlite-wal"),
        );

        // Photos writes while we copy: a database and -wal from two moments
        // read as damaged, or worse, as rows that never stood together. So the
        // copy counts only when neither file changed on the phone meanwhile.
        for _ in 0..3 {
            let before = moments(&mut afc).await;

            // Recent changes sit in the -wal file until Photos folds them in,
            // and SQLite replays it on open. Not the -shm: it is an index of
            // the -wal in shared memory, and SQLite rebuilds it.
            pull(&mut afc, PHOTOS_DB, &db, wanted)
                .await
                .doing("Photos.sqlite")?;
            let _ = fs::remove_file(&wal);
            if pull(&mut afc, PHOTOS_WAL, &wal, wanted).await.is_err() {
                let _ = fs::remove_file(&wal);
            }

            if moments(&mut afc).await == before {
                return read_photos(&db).doing("cannot read Photos.sqlite");
            }
        }

        Err("Photos kept changing while it was copied".to_string())
    }
    .await;

    let _ = fs::remove_dir_all(&scratch);

    read
}

/// A copy of Photos.sqlite left behind by a run that was killed: hundreds of
/// megabytes in a folder nobody looks in. One another window is making
/// changes every second; one this old belongs to nobody.
fn sweep_photos() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };

    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("rollport-photos-")
        {
            continue;
        }

        let old = |meta: std::io::Result<fs::Metadata>| {
            meta.and_then(|meta| meta.modified())
                .is_ok_and(|when| when.elapsed().is_ok_and(|age| age > ABANDONED))
        };

        // The folder too: one just made is empty until its copy begins.
        let abandoned = old(entry.metadata())
            && fs::read_dir(entry.path())
                .is_ok_and(|files| files.flatten().all(|file| old(file.metadata())));

        if abandoned {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

const PHOTOS_DB: &str = "/PhotoData/Photos.sqlite";
const PHOTOS_WAL: &str = "/PhotoData/Photos.sqlite-wal";

/// The size and change time of the database and its -wal, as far as AFC says.
async fn moments(afc: &mut AfcClient) -> [Option<(usize, chrono::NaiveDateTime)>; 2] {
    let mut seen = [None, None];
    for (slot, path) in seen.iter_mut().zip([PHOTOS_DB, PHOTOS_WAL]) {
        *slot = within("", afc.get_file_info(path))
            .await
            .ok()
            .map(|info| (info.size, info.modified));
    }
    seen
}

/// One file from the phone to the disk, whole.
/// Stops early, closing what it opened, once it is no longer `wanted`.
async fn pull(
    afc: &mut AfcClient,
    from: &str,
    to: &Path,
    wanted: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    let mut file = within("cannot open it", afc.open(from, AfcFopenMode::RdOnly)).await?;

    let read = async {
        let mut sink = tokio::fs::File::create(to).await.doing("cannot write it")?;
        for chunk in 0usize.. {
            if chunk % 16 == 0 && !wanted() {
                return Err("nobody is waiting for it".to_string());
            }
            let bytes = within("cannot read it", file.read_n(CHUNK)).await?;
            if bytes.is_empty() {
                break;
            }
            sink.write_all(&bytes).await.doing("cannot write it")?;
        }
        sink.flush().await.doing("cannot write it")
    }
    .await;

    // idevice asserts when an open file is dropped without being closed.
    within("cannot close it", file.close()).await?;

    read
}

/// Photos.sqlite holds a row per photo or film, under the folder and name it
/// has in /DCIM; a Live Photo's clip has none of its own. ZDATECREATED is
/// the time Photos shows, and moves when somebody adjusts it there. On an
/// iPhone with iOS 27 and 8,729 files, each file matched one row by this path,
/// and each .MOV with no row sat beside its Live Photo's still.
pub fn read_photos(path: &Path) -> rusqlite::Result<HashMap<String, Known>> {
    let db = rusqlite::Connection::open(path)?;

    // Columns come and go between versions of iOS, and the asset table was
    // ZGENERICASSET before iOS 14. A column this iOS lacks reads as empty, so
    // an older phone still gets what its database has.
    let columns = |table: &str| -> rusqlite::Result<HashSet<String>> {
        db.prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |row| row.get(1))?
            .collect()
    };
    let mut table = "ZASSET";
    let mut asset = columns(table)?;
    if asset.is_empty() {
        table = "ZGENERICASSET";
        asset = columns(table)?;
    }
    let extra = columns("ZADDITIONALASSETATTRIBUTES")?;
    let joined = extra.contains("ZASSET");

    let a = |name: &str| {
        if asset.contains(name) {
            format!("a.{name}")
        } else {
            "NULL".to_string()
        }
    };
    let x = |name: &str| {
        if joined && extra.contains(name) {
            format!("x.{name}")
        } else {
            "NULL".to_string()
        }
    };
    let join = if joined {
        "LEFT JOIN ZADDITIONALASSETATTRIBUTES x ON x.ZASSET = a.Z_PK"
    } else {
        ""
    };

    let (created, trashed, zone) = (a("ZDATECREATED"), a("ZTRASHEDSTATE"), x("ZTIMEZONEOFFSET"));
    let (burst, visibility, kind) = (
        a("ZAVALANCHEUUID"),
        a("ZVISIBILITYSTATE"),
        a("ZKINDSUBTYPE"),
    );
    let (adjusted, has_adjustments) = (a("ZADJUSTMENTSSTATE"), a("ZHASADJUSTMENTS"));
    let (original, cloud) = (x("ZORIGINALFILENAME"), a("ZCLOUDLOCALSTATE"));

    let mut rows = db.prepare(&format!(
        "SELECT a.ZDIRECTORY, a.ZFILENAME, {created}, {trashed}, {zone},
                {burst} IS NOT NULL AND {visibility} != 0, {kind},
                COALESCE({adjusted}, 0) > 0 OR COALESCE({has_adjustments}, 0) = 1, {original}
         FROM {table} a {join}
         WHERE (a.ZDIRECTORY LIKE 'DCIM/%'
                OR ({cloud} = 1 AND (a.ZDIRECTORY LIKE 'PhotoData/CPLAssets%'
                                     OR a.ZDIRECTORY LIKE 'CPLAssets%')))
           AND a.ZFILENAME IS NOT NULL"
    ))?;

    rows.query_map([], |row| {
        let folder: String = row.get(0)?;
        let file: String = row.get(1)?;
        // iCloud's store sits under /PhotoData, which some versions leave out.
        let folder = match folder.starts_with("CPLAssets") {
            true => format!("PhotoData/{folder}"),
            false => folder,
        };
        let path = format!("/{folder}/{file}");
        let created: Option<f64> = row.get(2)?;
        let trashed: Option<i64> = row.get(3)?;
        let offset: Option<i64> = row.get(4)?;
        // A burst keeps every frame in /DCIM, and Photos shows the picked
        // ones: 0 is shown, 2 is not (as osxphotos reads it).
        let unpicked: Option<bool> = row.get(5)?;
        let kind = match row.get::<_, Option<i64>>(6)? {
            Some(2) => Some(Kind::LivePhoto),
            Some(10) => Some(Kind::Screenshot),
            Some(103) => Some(Kind::ScreenRecording),
            _ => None,
        };
        let edited: Option<bool> = row.get(7)?;
        let original: Option<String> = row.get(8)?;

        if trashed.unwrap_or(0) != 0 || unpicked.unwrap_or(false) {
            return Ok(Some((path, Known::Hidden)));
        }

        let utc = created
            .and_then(|seconds| {
                chrono::DateTime::from_timestamp(seconds.floor() as i64 + APPLE_EPOCH, 0)
            })
            .map(|utc| utc.naive_utc());

        let Some(utc) = utc else {
            return Ok(None);
        };

        let taken = offset
            .and_then(|seconds| i32::try_from(seconds).ok())
            .and_then(chrono::FixedOffset::east_opt)
            .map_or_else(|| here(utc), |zone| utc.and_utc().with_timezone(&zone));

        if !path.starts_with("/DCIM/") {
            let name = original.filter(|name| !name.is_empty()).unwrap_or(file);
            return Ok(Some((path, Known::Kept { taken, kind, name })));
        }

        let edited = edited.unwrap_or(false);
        Ok(Some((
            path,
            Known::Taken {
                taken,
                kind,
                edited,
            },
        )))
    })?
    .filter_map(Result::transpose)
    .collect()
}

/// Each file gets the time Photos shows for it, and what is in Recently
/// Deleted goes, with its Live Photo clip, which has no row of its own.
///
/// So does a photo the phone has not finished. Seconds after a shot, /DCIM
/// can hold IMG_2917D.HEIC beside IMG_2917.MOV while Photos' row already
/// names IMG_2917.HEIC, the photo it will become; the clip only pairs with
/// that. Both stay on the phone until the photo is finished, and the next
/// import takes the pair.
pub fn settle(media: &mut Vec<Media>, known: &HashMap<String, Known>) {
    let mut held: HashSet<String> = known
        .iter()
        .filter(|(_, known)| **known == Known::Hidden)
        .map(|(path, _)| stem(path))
        .collect();

    let named: HashSet<String> = known.keys().map(|path| stem(path)).collect();
    for file in media.iter().filter(|file| !known.contains_key(&file.path)) {
        if let Some(finished) = stem(&file.path).strip_suffix('d')
            && named.contains(finished)
        {
            held.insert(finished.to_string());
            held.insert(stem(&file.path));
        }
    }

    media.retain(|file| !held.contains(&stem(&file.path)));

    for file in media.iter_mut() {
        if let Some(Known::Taken { taken, kind, .. }) = known.get(&file.path) {
            file.taken = *taken;
            file.kind = kind.unwrap_or(file.kind);
        }
    }
}

/// A .mov beside an image of the same name is the video half of a Live Photo,
/// and carries the image's time. The image is a Live Photo, also where Photos
/// does not say so, as on older iOS.
pub fn mark_live_videos(media: &mut [Media]) {
    let still = |file: &Media| {
        matches!(
            extension(&file.path).as_deref(),
            Some("heic" | "heif" | "jpg" | "jpeg" | "dng")
        )
    };
    let movie = |file: &Media| extension(&file.path).as_deref() == Some("mov");

    let images: HashMap<String, chrono::DateTime<chrono::FixedOffset>> = media
        .iter()
        .filter(|file| still(file))
        .map(|file| (stem(&file.path), file.taken))
        .collect();
    let clips: HashSet<String> = media
        .iter()
        .filter(|file| movie(file) && images.contains_key(&stem(&file.path)))
        .map(|file| stem(&file.path))
        .collect();

    for file in media.iter_mut() {
        if !clips.contains(&stem(&file.path)) {
            continue;
        }
        if movie(file) {
            file.kind = Kind::LiveVideo;
            file.taken = images[&stem(&file.path)];
        } else if still(file) && file.kind == Kind::Photo {
            file.kind = Kind::LivePhoto;
        }
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

/// Apple's name for an edited copy: IMG_E2739.JPG for IMG_2739.PNG's render
/// in JPEG.
pub fn edited_name(original: &str, render: &str) -> String {
    let stem = original.rsplit_once('.').map_or(original, |(stem, _)| stem);
    let extension = extension(render).unwrap_or_default().to_uppercase();
    match stem.split_once('_') {
        Some((prefix, number)) => format!("{prefix}_E{number}.{extension}"),
        None => format!("{stem}_E.{extension}"),
    }
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
    // What the window counts as done and imported: photos and videos as
    // Batch counts them, or every file when they all ride along.
    let shown_total = Batch::of(fetch.iter()).items;
    let all_ride_along = fetch.iter().all(Media::rides_along);
    let counts = |file: &Media| all_ride_along || !file.rides_along();
    let mut shown_done = 0;
    let total_bytes = fetch.iter().map(|file| file.size).sum::<u64>();
    let started = Instant::now();

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
    let mut lost = Lost::default();
    let mut bytes = 0;
    // What was on disk already, and so never streams.
    let mut skipped = 0;
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

    // Said on a clock, not as files finish: one big video can take minutes.
    let mut tick = tokio::time::interval(ANNOUNCE);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let finished = tokio::select! {
            finished = rx.recv() => match finished {
                Some(finished) => Some(finished),
                None => break,
            },
            _ = tick.tick() => None,
        };

        if let Some((index, outcome)) = finished {
            let file = &fetch[index];

            done += 1;
            shown_done += usize::from(counts(file));
            bytes += file.size;

            let recorded = match outcome {
                Ok(Copied::Fetched) => {
                    landed.push(file);

                    // A Live Photo's clip has no thumbnail; its still stands for it.
                    if !file.is_live_video() && !file.is_edit() {
                        latest = Some(file.path.clone());
                    }

                    library.record(file)
                }
                // It was already on disk but the folder had never been told.
                // Recording it is what stops it being offered as new for ever.
                Ok(Copied::Already) => {
                    skipped += file.size;
                    library.record(file)
                }
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
                    lost.add(&file.name, reason, cancel.load(Ordering::Relaxed));
                    Ok(())
                }
                Err(Failed::TooLarge) => {
                    let reason = "too large for this drive: FAT32 drives hold files up to 4 GB";
                    lost.add(&file.name, reason.into(), cancel.load(Ordering::Relaxed));
                    Ok(())
                }
            };

            if let Err(reason) = recorded {
                cancel.store(true, Ordering::Relaxed);
                fatal.get_or_insert(reason);
            }

            // The last announcement waits for no clock.
            if done < total {
                continue;
            }
        }

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
        let gone = arrived + skipped;
        let fresh = (rate > 1.0).then(|| total_bytes.saturating_sub(gone) as f64 / rate);
        let now = started.elapsed().as_secs_f64();
        quote = requote(quote, fresh, now);

        with(app, |state| {
            state.progress(Progress {
                done: shown_done,
                total: shown_total,
                // A retried file streams twice; a failed one may never stream.
                bytes: bytes.max(gone).min(total_bytes),
                total_bytes,
                eta: quote.map(|(left, at)| (left - (now - at)).max(0.0) as u64),
                latest: latest.clone(),
            })
        });
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
    let landed = Batch::of(landed);

    Ok(Report {
        imported: landed.items,
        bytes: landed.bytes,
        seconds: started.elapsed().as_secs(),
        end,
        newest: landed.newest,
        udid: udid.to_string(),
        // One unreadable file is not a reason to abandon the other nine
        // thousand, so what it was is reported at the end of the run.
        trouble: lost.trouble(),
        // Written by whoever started the run, who knows what it ran on.
        details: String::new(),
    })
}

/// The files that did not arrive, by name, with what each said. Which file,
/// and what it said, is the difference between one bad photo and a phone that
/// locked halfway through.
#[derive(Default)]
struct Lost(Vec<(String, String)>);

impl Lost {
    /// One file that did not arrive. After Stop it is not lost: the stop
    /// broke it off, the phone still has all of it, and the folder takes it
    /// next time. Nor after a broken folder, which raises the stop flag too
    /// and is said once, as the run's end.
    fn add(&mut self, name: &str, reason: String, stopped: bool) {
        if !stopped {
            self.0.push((name.to_string(), reason));
        }
    }

    fn trouble(self) -> Option<Trouble> {
        (!self.0.is_empty()).then_some(Trouble(self.0))
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

/// The name a file is copied under until it is whole: where it lives on the
/// phone, which no other file shares, and this process, because another copy
/// of the app may be fetching the same photo into the same folder.
fn scratch_name(path: &str) -> String {
    format!(
        ".{}.{}.tmp",
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
        // flat layout every photo in the library is in this listing. Edits
        // and iCloud's originals come from /PhotoData; .part is from 0.1.0,
        // which OneDrive uploaded, as it skips .tmp.
        let ours = (name.starts_with(".DCIM_") || name.starts_with(".PhotoData_"))
            && name.ends_with(".tmp")
            || name.starts_with(".DCIM_") && name.ends_with(".part");
        if !ours {
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
#[derive(Debug)]
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
    /// Larger than one file on this disk may be: FAT32 stops at 4 GB. No
    /// other try will fit it, and the rest of the camera roll still does.
    TooLarge,
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
            std::io::ErrorKind::FileTooLarge => Failed::TooLarge,
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
    let name = &file.name;
    let rendered = render(layout, name, file.taken.naive_local());

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
    // so the time has to match too, and we set that on every copy.
    // The CLI and 0.1.0 named and dated a file by when it was written on the
    // phone, in this computer's zone, so a folder they filled has it under
    // that name. A Live Photo's clip was named by its still's time, which is
    // not known here.
    // The legacy CLI never made this claim at all; it suffixed and downloaded
    // again. This is that, minus re-downloading what is demonstrably here.
    let named_before = destination.join(render(layout, name, here(file.mtime).naive_local()));

    if holds(&target, file) || (!file.is_live_video() && holds(&named_before, file)) {
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
            "only {written} of {} bytes arrived",
            file.size
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
    // sweep of another window. A file left with today's date is not one the
    // next run can recognise as already here, so it does not stay; the next
    // try fetches it again.
    if let Err(e) = stamp(&target, file.taken) {
        let _ = fs::remove_file(&target);
        return Err(Failed::File(format!(
            "cannot set the date of the file: {e}"
        )));
    }

    Ok(Copied::Fetched)
}

/// Whether this file on disk is that one from the phone: the same size, and
/// dated with either of its times to the second.
fn holds(path: &Path, file: &Media) -> bool {
    fs::metadata(path).is_ok_and(|on_disk| {
        on_disk.len() == file.size
            && on_disk.modified().is_ok_and(|when| {
                let second = chrono::DateTime::<chrono::Utc>::from(when).timestamp();
                second == file.taken.timestamp() || second == file.mtime.and_utc().timestamp()
            })
    })
}

/// Dates a file: when it was changed, and on macOS and Windows when it was
/// created, which Explorer and Windows Photos sort by. Linux keeps no
/// settable creation date. Windows asks only for the right to change its
/// attributes: asking to write it is refused while OneDrive, the indexer or
/// a virus scanner has the new file open.
fn stamp(path: &Path, when: chrono::DateTime<chrono::FixedOffset>) -> std::io::Result<()> {
    let mut options = fs::File::options();
    options.write(true);

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.access_mode(windows_sys::Win32::Storage::FileSystem::FILE_WRITE_ATTRIBUTES);
    }

    let when = SystemTime::from(when);
    let times = fs::FileTimes::new().set_modified(when);

    #[cfg(target_os = "macos")]
    let times = std::os::macos::fs::FileTimesExt::set_created(times, when);
    #[cfg(windows)]
    let times = std::os::windows::fs::FileTimesExt::set_created(times, when);

    options.open(path)?.set_times(times)
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
            Err(e) => return Err(e).on_disk("cannot write the file"),
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
                if let Device::Waiting { need: Need::Failed { reason }, .. } = &status {
                    crate::note(app, &format!("cannot connect to the iPhone: {reason}"));
                }
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
    if let Devices::Unavailable { reason } = &devices {
        crate::note(app, &format!("cannot see iPhones: {reason}"));
    }
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

    /// A file's own name, without the folders it sat in.
    fn name_of(path: &str) -> &str {
        path.rsplit('/').next().unwrap_or(path)
    }

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
    fn a_file_is_lost_only_while_the_run_goes_on() {
        let mut lost = Lost::default();
        lost.add("IMG_0001.HEIC", "timed out".into(), false);
        lost.add("IMG_0002.HEIC", "gone".into(), false);
        lost.add("IMG_0003.HEIC", "stopped".into(), true);

        let trouble = lost.trouble().expect("two were lost");
        assert_eq!(
            trouble.0,
            [
                ("IMG_0001.HEIC".to_string(), "timed out".to_string()),
                ("IMG_0002.HEIC".to_string(), "gone".to_string()),
            ]
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
            name: name_of(path).into(),
            size: 1,
            mtime: when("2026-09-02 15:04:11"),
            taken: here(when("2026-09-02 15:04:11")),
            kind: Kind::Photo,
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

        let kinds: Vec<Kind> = media.iter().map(|file| file.kind).collect();
        // Only the .mov sharing a name with an image in its own folder, and
        // that image, even where Photos gave no kind.
        assert_eq!(
            kinds,
            [
                Kind::LivePhoto,
                Kind::LiveVideo,
                Kind::Photo,
                Kind::Photo,
                Kind::LivePhoto,
                Kind::LiveVideo
            ]
        );
    }

    /// The clip is written a second or more after its still. Named with its
    /// own time, the pair would get two names, and other apps would show a
    /// photo and a separate video.
    #[test]
    fn a_live_photo_clip_is_named_and_dated_with_its_still() {
        let file = |path: &str, at: &str| Media {
            path: path.into(),
            name: name_of(path).into(),
            size: 1,
            mtime: when(at),
            taken: when(at).and_utc().fixed_offset(),
            kind: Kind::Photo,
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
            .map(|file| render(layout, &file.name, file.taken.naive_local()))
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

    /// The tables and columns as iOS 27 has them, with the rows a camera roll
    /// gives: a Live Photo from a trip, an edited photo saved from a chat years
    /// after it was taken, a screenshot with no zone, one in Recently Deleted,
    /// a burst, and a screen recording.
    fn photos_sqlite(dir: &Path) -> PathBuf {
        let path = dir.join("Photos.sqlite");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY VARCHAR,
                 ZFILENAME VARCHAR, ZDATECREATED TIMESTAMP, ZTRASHEDSTATE INTEGER,
                 ZAVALANCHEUUID VARCHAR, ZVISIBILITYSTATE INTEGER, ZKINDSUBTYPE INTEGER,
                 ZADJUSTMENTSSTATE INTEGER);
             CREATE TABLE ZADDITIONALASSETATTRIBUTES (Z_PK INTEGER PRIMARY KEY,
                 ZASSET INTEGER, ZTIMEZONEOFFSET INTEGER);
             INSERT INTO ZASSET VALUES
                 (1, 'DCIM/100APPLE', 'IMG_0001.HEIC', 810000000.25, 0, NULL, 0, 2, 0),
                 (2, 'DCIM/100APPLE', 'IMG_0002.JPG', 560000000, 0, NULL, 0, 0, 3),
                 (3, 'DCIM/100APPLE', 'IMG_0003.PNG', 810000000, NULL, NULL, 0, 10, NULL),
                 (4, 'DCIM/100APPLE', 'IMG_0004.HEIC', 810000000, 1, NULL, 0, 0, 0),
                 (5, 'PhotoData/CPLAssets', 'IMG_0005.HEIC', 810000000, 0, NULL, 0, 0, 0),
                 (6, 'DCIM/100APPLE', 'IMG_0006.HEIC', 810000000, 0, 'b', 0, 0, 0),
                 (7, 'DCIM/100APPLE', 'IMG_0007.HEIC', 810000000, 0, 'b', 2, 0, 0),
                 (8, 'DCIM/100APPLE', 'IMG_0008.MP4', 810000000, 0, NULL, 0, 103, 0);
             INSERT INTO ZADDITIONALASSETATTRIBUTES VALUES
                 (1, 1, 32400), (2, 2, 3600), (3, 3, NULL), (4, 4, 3600);",
        )
        .unwrap();
        path
    }

    #[test]
    fn photos_dates_each_file_where_it_was_taken() {
        let dir = std::env::temp_dir().join(format!("rollport-dates-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let known = read_photos(&photos_sqlite(&dir)).unwrap();
        let _ = fs::remove_dir_all(&dir);

        let at = |text: &str, kind, edited| Known::Taken {
            taken: chrono::DateTime::parse_from_rfc3339(text).unwrap(),
            kind,
            edited,
        };

        // 810000000 seconds after 2001-01-01 is 2026-09-02 00:00:00 UTC.
        assert_eq!(
            known["/DCIM/100APPLE/IMG_0001.HEIC"],
            at("2026-09-02T09:00:00+09:00", Some(Kind::LivePhoto), false)
        );
        assert_eq!(
            known["/DCIM/100APPLE/IMG_0002.JPG"],
            at("2018-09-30T12:33:20+01:00", None, true)
        );
        assert!(matches!(
            known["/DCIM/100APPLE/IMG_0003.PNG"],
            Known::Taken {
                kind: Some(Kind::Screenshot),
                ..
            }
        ));
        assert!(matches!(
            known["/DCIM/100APPLE/IMG_0008.MP4"],
            Known::Taken {
                kind: Some(Kind::ScreenRecording),
                ..
            }
        ));
        assert_eq!(known["/DCIM/100APPLE/IMG_0004.HEIC"], Known::Hidden);
        // A burst: the picked frame, and one that was not.
        assert!(matches!(
            known["/DCIM/100APPLE/IMG_0006.HEIC"],
            Known::Taken { .. }
        ));
        assert_eq!(known["/DCIM/100APPLE/IMG_0007.HEIC"], Known::Hidden);
        // No zone: the same instant, in this machine's zone.
        let Known::Taken { taken: no_zone, .. } = known["/DCIM/100APPLE/IMG_0003.PNG"] else {
            panic!("a photo with no zone is still dated")
        };
        assert_eq!(no_zone.timestamp(), 810_000_000 + APPLE_EPOCH);
        // Only what is in /DCIM.
        assert_eq!(known.len(), 7);
    }

    /// Before iOS 14 the table was ZGENERICASSET, and an old database lacks
    /// columns a new one has. What it has is still read.
    #[test]
    fn an_older_iphones_photos_still_dates_its_files() {
        let dir = std::env::temp_dir().join(format!("rollport-old-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Photos.sqlite");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE ZGENERICASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY VARCHAR,
                     ZFILENAME VARCHAR, ZDATECREATED TIMESTAMP, ZKINDSUBTYPE INTEGER,
                     ZHASADJUSTMENTS INTEGER);
                 INSERT INTO ZGENERICASSET VALUES
                     (1, 'DCIM/100APPLE', 'IMG_0001.PNG', 810000000, 10, 1);",
            )
            .unwrap();

        let known = read_photos(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);

        let Known::Taken {
            taken,
            kind,
            edited,
        } = known["/DCIM/100APPLE/IMG_0001.PNG"]
        else {
            panic!("an old database still dates its photos")
        };
        assert_eq!(taken.timestamp(), 810_000_000 + APPLE_EPOCH);
        assert_eq!(kind, Some(Kind::Screenshot));
        assert!(edited);
    }

    /// With iCloud Photos, the originals of photos from other devices that
    /// the phone keeps sit in iCloud's store, named by an id.
    #[test]
    fn icloud_originals_kept_on_the_phone_are_read_with_their_names() {
        let dir = std::env::temp_dir().join(format!("rollport-cpl-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Photos.sqlite");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY VARCHAR,
                     ZFILENAME VARCHAR, ZDATECREATED TIMESTAMP, ZCLOUDLOCALSTATE INTEGER);
                 CREATE TABLE ZADDITIONALASSETATTRIBUTES (Z_PK INTEGER PRIMARY KEY,
                     ZASSET INTEGER, ZORIGINALFILENAME VARCHAR);
                 INSERT INTO ZASSET VALUES
                     (1, 'PhotoData/CPLAssets/group7', 'A1B2.JPG', 810000000, 1),
                     (2, 'CPLAssets/group3', 'C3D4.HEIC', 810000000, 1),
                     (3, 'PhotoData/CPLAssets/group7', 'E5F6.JPG', 810000000, 0),
                     (4, 'PhotoData/Sync/100SYNCD', 'IMG_0001.JPG', 810000000, 1);
                 INSERT INTO ZADDITIONALASSETATTRIBUTES VALUES
                     (1, 1, 'IMG_1234.JPG'), (2, 2, NULL);",
            )
            .unwrap();

        let known = read_photos(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);

        let name = |path: &str| match &known[path] {
            Known::Kept { name, .. } => name.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(name("/PhotoData/CPLAssets/group7/A1B2.JPG"), "IMG_1234.JPG");
        assert_eq!(name("/PhotoData/CPLAssets/group3/C3D4.HEIC"), "C3D4.HEIC");
        // One only in iCloud, and photos synced from a computer, stay out.
        assert_eq!(known.len(), 2);
    }

    #[test]
    fn an_edit_is_named_as_apple_names_an_edited_copy() {
        assert_eq!(
            edited_name("IMG_2739.PNG", "a/FullSizeRender.jpg"),
            "IMG_E2739.JPG"
        );
        assert_eq!(
            edited_name("IMG_2917.MOV", "a/FullSizeRender.mov"),
            "IMG_E2917.MOV"
        );
        assert_eq!(
            edited_name("DSC0001.HEIC", "a/FullSizeRender.heic"),
            "DSC0001_E.HEIC"
        );
    }

    #[test]
    fn settling_dates_by_photos_and_leaves_out_recently_deleted() {
        let file = |path: &str| Media {
            path: path.into(),
            name: name_of(path).into(),
            size: 1,
            mtime: when("2026-11-02 10:00:00"),
            taken: here(when("2026-11-02 10:00:00")),
            kind: Kind::Photo,
        };
        let mut media = vec![
            file("/DCIM/100APPLE/IMG_0001.HEIC"),
            file("/DCIM/100APPLE/IMG_0001.MOV"),
            file("/DCIM/100APPLE/IMG_0004.HEIC"),
            file("/DCIM/100APPLE/IMG_0004.MOV"),
            // Not finished yet: Photos names the photo it will become.
            file("/DCIM/100APPLE/IMG_0005D.HEIC"),
            file("/DCIM/100APPLE/IMG_0005.MOV"),
            file("/DCIM/100APPLE/IMG_0009.JPG"),
            // A D of its own, with no finished photo to wait for.
            file("/DCIM/100APPLE/IMG_0010D.JPG"),
        ];
        let taken = chrono::DateTime::parse_from_rfc3339("2024-05-01T08:00:00+02:00").unwrap();
        let known = HashMap::from([
            (
                "/DCIM/100APPLE/IMG_0001.HEIC".to_string(),
                Known::Taken {
                    taken,
                    kind: None,
                    edited: false,
                },
            ),
            ("/DCIM/100APPLE/IMG_0004.HEIC".to_string(), Known::Hidden),
            (
                "/DCIM/100APPLE/IMG_0005.HEIC".to_string(),
                Known::Taken {
                    taken,
                    kind: None,
                    edited: false,
                },
            ),
        ]);

        settle(&mut media, &known);
        mark_live_videos(&mut media);

        let dated: Vec<(&str, String)> = media
            .iter()
            .map(|file| (file.path.as_str(), file.taken.to_rfc3339()))
            .collect();
        assert_eq!(
            dated,
            [
                ("/DCIM/100APPLE/IMG_0001.HEIC", taken.to_rfc3339()),
                // The clip of a Live Photo takes its still's time.
                ("/DCIM/100APPLE/IMG_0001.MOV", taken.to_rfc3339()),
                // Photos has no row for it: when it was written.
                (
                    "/DCIM/100APPLE/IMG_0009.JPG",
                    here(when("2026-11-02 10:00:00")).to_rfc3339()
                ),
                (
                    "/DCIM/100APPLE/IMG_0010D.JPG",
                    here(when("2026-11-02 10:00:00")).to_rfc3339()
                ),
            ]
        );
    }

    /// Against the phone on the cable: Photos' record is read, and a file
    /// written on the phone long after it was taken lands in the year and
    /// month it was taken, with that date on disk.
    /// `cargo test --lib real_phone -- --ignored --nocapture`
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs a trusted iPhone on the cable"]
    async fn real_phone() {
        let addr = UsbmuxdAddr::from_env_var().unwrap();
        let udid = addr
            .connect(0)
            .await
            .unwrap()
            .get_devices()
            .await
            .unwrap()
            .into_iter()
            .find(|device| device.connection_type == Connection::Usb)
            .expect("an iPhone on the cable")
            .udid;

        let started = Instant::now();
        let known = photos(&udid, 0, &|| true).await.unwrap();
        let hidden = known.values().filter(|k| **k == Known::Hidden).count();
        eprintln!(
            "Photos.sqlite: {} files, {hidden} hidden, {:.1}s",
            known.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(known.len() > hidden);
        let mut kinds = HashMap::new();
        for known in known.values() {
            if let Known::Taken {
                kind: Some(kind), ..
            } = known
            {
                *kinds.entry(format!("{kind:?}")).or_insert(0) += 1;
            }
        }
        eprintln!("kinds Photos names: {kinds:?}");

        let mut afc = files_of(&udid).await.unwrap();

        // A copy nobody waits for stops, and leaves the connection usable.
        let scratch =
            std::env::temp_dir().join(format!("rollport-unwanted-{}", std::process::id()));
        let stopped = pull(&mut afc, "/PhotoData/Photos.sqlite", &scratch, &|| false).await;
        let _ = fs::remove_file(&scratch);
        assert!(stopped.is_err());

        let mut late = None;
        for (path, known) in &known {
            let Known::Taken { taken, .. } = known else {
                continue;
            };
            let info = afc.get_file_info(path).await.unwrap();
            if (info.modified - taken.naive_utc()).num_days() > 30 {
                late = Some(Media {
                    name: name_of(path).into(),
                    path: path.clone(),
                    size: info.size as u64,
                    mtime: info.modified,
                    taken: *taken,
                    kind: Kind::Photo,
                });
                break;
            }
        }
        let file = late.expect("a file written more than 30 days after it was taken");
        eprintln!("{} written {} taken {}", file.path, file.mtime, file.taken);

        let dir = std::env::temp_dir().join(format!("rollport-real-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let layout = "{mtime:%Y}/{mtime:%m}/{name}";
        let copied = copy(
            &mut afc,
            &file,
            &dir,
            layout,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            &Mutex::new(0),
        )
        .await;
        assert!(matches!(copied, Ok(Copied::Fetched)), "{copied:?}");

        let target = dir.join(render(layout, &file.name, file.taken.naive_local()));
        let on_disk = fs::metadata(&target).unwrap().modified().unwrap();
        eprintln!("landed at {}", target.display());
        assert_eq!(on_disk, SystemTime::from(file.taken));

        // Copied again, it is recognised as already there.
        let again = copy(
            &mut afc,
            &file,
            &dir,
            layout,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            &Mutex::new(0),
        )
        .await;
        assert!(matches!(again, Ok(Copied::Already)));

        // A folder 0.1.0 filled: the file under the name and date of when it
        // was written on the phone.
        fs::remove_dir_all(&dir).unwrap();
        let before = dir.join(render(layout, &file.name, here(file.mtime).naive_local()));
        fs::create_dir_all(before.parent().unwrap()).unwrap();
        fs::write(&before, vec![0; file.size as usize]).unwrap();
        stamp(&before, file.mtime.and_utc().fixed_offset()).unwrap();
        let filled = copy(
            &mut afc,
            &file,
            &dir,
            layout,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            &Mutex::new(0),
        )
        .await;
        eprintln!("0.1.0 named it {}", before.display());
        assert!(matches!(filled, Ok(Copied::Already)), "{filled:?}");
        assert!(!target.exists());

        // An edited photo: its render comes beside it, named as Apple names
        // an edited copy, and dated with the original.
        let (path, taken) = known
            .iter()
            .find_map(|(path, known)| match known {
                Known::Taken {
                    taken,
                    edited: true,
                    ..
                } => Some((path.clone(), *taken)),
                _ => None,
            })
            .expect("an edited photo on this iPhone");
        let info = afc.get_file_info(&path).await.unwrap();
        let original = Media {
            kind: Kind::of(&path),
            name: name_of(&path).into(),
            path,
            size: info.size as u64,
            mtime: info.modified,
            taken,
        };
        let found = edits(&mut afc, &original).await.unwrap();
        let edit = found.first().expect("its render");
        let copied = copy(
            &mut afc,
            edit,
            &dir,
            layout,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            &Mutex::new(0),
        )
        .await;
        assert!(matches!(copied, Ok(Copied::Fetched)), "{copied:?}");
        let landed = dir.join(render(layout, &edit.name, taken.naive_local()));
        eprintln!("{} edited, landed at {}", original.path, landed.display());
        assert_eq!(
            fs::metadata(&landed).unwrap().modified().unwrap(),
            SystemTime::from(taken)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_on_disk_is_the_one_from_the_phone_by_size_and_either_time() {
        let dir = std::env::temp_dir().join(format!("rollport-holds-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("IMG_0001.HEIC");
        fs::write(&path, [0; 42]).unwrap();

        let file = Media {
            path: "/DCIM/100APPLE/IMG_0001.HEIC".into(),
            name: "IMG_0001.HEIC".into(),
            size: 42,
            mtime: when("2026-11-02 10:00:00"),
            // Before 1970: a date somebody set in Photos.
            taken: chrono::DateTime::parse_from_rfc3339("1965-05-01T08:00:00+01:00").unwrap(),
            kind: Kind::Photo,
        };
        let dated = |at: chrono::DateTime<chrono::FixedOffset>| {
            stamp(&path, at).unwrap();
            holds(&path, &file)
        };

        assert!(dated(file.taken));
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt;
            let meta = fs::metadata(&path).unwrap();
            assert_eq!(meta.st_birthtime(), file.taken.timestamp());
        }
        assert!(dated(file.mtime.and_utc().fixed_offset()));
        assert!(!dated(file.taken + chrono::Duration::seconds(1)));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_abandoned_copy_of_photos_goes_and_one_being_made_stays() {
        let dir = |age| {
            let dir = std::env::temp_dir()
                .join(format!("rollport-photos-test-{age}-{}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let file = fs::File::create(dir.join("Photos.sqlite")).unwrap();
            file.set_modified(SystemTime::now() - Duration::from_secs(age))
                .unwrap();
            fs::File::open(&dir)
                .and_then(|folder| {
                    folder.set_modified(SystemTime::now() - Duration::from_secs(age))
                })
                .unwrap();
            dir
        };
        let (old, fresh) = (dir(3600), dir(5));

        sweep_photos();

        assert!(!old.exists());
        assert!(fresh.exists());
        let _ = fs::remove_dir_all(fresh);
    }

    /// Past 4 GB on a FAT32 drive the write fails, and that is this file's
    /// failure, not the folder's. Needs a FAT32 volume mounted at
    /// ROLLPORT_FAT32, such as a disk image:
    /// `hdiutil create -size 5g -fs "MS-DOS FAT32" -volname FAT -type SPARSE fat`
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs a FAT32 volume"]
    async fn a_file_too_large_for_fat32_is_that_files_failure() {
        let path = Path::new(&std::env::var("ROLLPORT_FAT32").unwrap()).join("big.tmp");
        let mut sink = tokio::fs::File::create(&path).await.unwrap();
        let chunk = vec![0u8; CHUNK];

        let written = async {
            for _ in 0..=4096 {
                sink.write_all(&chunk).await?;
            }
            sink.flush().await
        }
        .await
        .on_disk("cannot write the file");

        let _ = fs::remove_file(&path);
        assert!(matches!(written, Err(Failed::TooLarge)), "{written:?}");
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
        let edit = scratch_name(
            "/PhotoData/Mutations/DCIM/100APPLE/IMG_0001/Adjustments/FullSizeRender.jpg",
        );
        make(&dead, Duration::from_secs(3600));
        make(&edit, Duration::from_secs(3600));
        make(&live, Duration::from_secs(5));
        let from_0_1_0 = ".DCIM_100APPLE_IMG_0003.HEIC.4242.part";
        make(from_0_1_0, Duration::from_secs(3600));
        make(".film.mp4.part", Duration::from_secs(3600));

        sweep(&dir);

        assert!(!dir.join(&dead).exists());
        assert!(!dir.join(&edit).exists());
        assert!(!dir.join(from_0_1_0).exists());
        assert!(dir.join(&live).exists());
        assert!(dir.join(".film.mp4.part").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
