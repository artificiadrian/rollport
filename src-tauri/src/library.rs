use std::{collections::HashSet, path::Path};

use crate::{
    Context,
    state::{FolderFacts, Media, Range, Settings, Takes, default_layout, local},
};

/// Identity is where it lived on the phone, its size, and the whole second it
/// last changed — so moving or renaming the local copy afterwards does not
/// import it again. Whole seconds, because the CLI writes the fraction too.
pub type Key = (String, u64, i64);

pub fn key(file: &Media) -> Key {
    (
        file.path.clone(),
        file.size,
        file.mtime.and_utc().timestamp(),
    )
}

/// What an import leaves free on the disk. At 0 bytes macOS and the apps
/// beside us start to fail, and one iPhone video can be over 1 GB.
const KEEP_FREE: u64 = 2_000_000_000;

/// Room an import may use on the disk the folder sits on. Huge when the disk
/// will not say.
pub fn room(path: &Path) -> u64 {
    fs4::available_space(path).map_or(u64::MAX, |free| free.saturating_sub(KEEP_FREE))
}

/// What a folder says about itself. Every answer about importing is the
/// folder's own, so it is read back from it rather than remembered: after an
/// import it is a library, and it holds more than it did.
pub fn read_folder(path: &Path) -> Result<FolderFacts, String> {
    // An unplugged drive, which `claimed` would call a folder with no rollport-library.db.
    // Import would fail at the first file; the window should say it now.
    if !path.is_dir() {
        return Err("cannot find the folder".into());
    }

    let plain = FolderFacts {
        keys: HashSet::new(),
        last: None,
        free: room(path),
        network: on_network(path),
        settings: Settings::default(),
    };

    // Not a library. Its answers are ours to change until the first import,
    // and nothing has been written into it.
    if !Library::claimed(path)? {
        return Ok(plain);
    }

    let (keys, last) = keys(path)?;
    let library = Library::open(path)?;

    Ok(FolderFacts {
        keys,
        last,
        settings: Settings {
            layout: library.setting("layout")?.unwrap_or_else(default_layout),
            takes: Takes::read(&library.setting("takes")?.unwrap_or_default()),
        },
        ..plain
    })
}

/// What the folder has taken, and when it last took anything. A folder with no
/// rollport-library.db has taken nothing, and reading it makes none.
///
/// An error is not an empty set: "not imported" would fetch every file again
/// under a `_1` name.
fn keys(folder: &Path) -> Result<(HashSet<Key>, Option<i64>), String> {
    if !Library::claimed(folder)? {
        return Ok((HashSet::new(), None));
    }

    let library = Library::open(folder)?;

    // CAST floors the CLI's float to the second, as `key` does.
    let keys = library
        .0
        .prepare("SELECT afc_path, st_size, CAST(st_mtime AS INTEGER) FROM media")
        .and_then(|mut sql| {
            sql.query_map([], |row| {
                Ok((row.get(0)?, row.get::<_, i64>(1)? as u64, row.get(2)?))
            })?
            .collect::<Result<HashSet<Key>, _>>()
        })
        .doing("cannot read rollport-library.db")?;

    // The CLI's schema stamps every row with the moment it was recorded, so
    // the folder already knows when it last took anything.
    let last = library
        .0
        .query_row("SELECT max(synced_at) FROM media", [], |row| {
            row.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten()
        .and_then(|stamped| {
            chrono::NaiveDateTime::parse_from_str(&stamped, "%Y-%m-%d %H:%M:%S").ok()
        })
        .map(|when| when.and_utc().timestamp());

    Ok((keys, last))
}

/// The files of this camera roll the folder takes and has never taken.
pub fn new<'a>(
    media: &'a [Media],
    takes: Takes,
    range: Range,
    keys: &'a HashSet<Key>,
) -> impl Iterator<Item = &'a Media> {
    media
        .iter()
        .filter(move |file| takes.takes(file))
        // The dates picked are days in this timezone, and the file is named
        // with its local day too, so the filter compares the same thing. AFC
        // reports UTC.
        .filter(move |file| range.takes(local(file.mtime).date()))
        .filter(|file| !keys.contains(&key(file)))
}

/// Whether the folder sits on a network drive. rollport-library.db runs in SQLite's WAL
/// mode, which does not work over a network file system, and a copy there can
/// fail halfway when the link drops.
#[cfg(target_os = "macos")]
fn on_network(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();

    // SAFETY: `path` ends in NUL, and `info` is read only after statfs filled it.
    unsafe {
        libc::statfs(path.as_ptr(), info.as_mut_ptr()) == 0
            && info.assume_init().f_flags & libc::MNT_LOCAL as u32 == 0
    }
}

/// The same question on Windows: a `\\server\share` path, or a drive letter
/// mapped to one.
#[cfg(windows)]
fn on_network(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    use windows_sys::Win32::{
        Storage::FileSystem::GetDriveTypeW, System::WindowsProgramming::DRIVE_REMOTE,
    };

    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };

    match prefix.kind() {
        Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            let root: Vec<u16> = format!("{}:\\", letter as char)
                .encode_utf16()
                .chain([0])
                .collect();

            // SAFETY: `root` is a NUL-terminated wide string.
            unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
        }
        _ => false,
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn on_network(_: &Path) -> bool {
    false
}

pub struct Library(rusqlite::Connection);

impl Library {
    /// The folder is a library once rollport-library.db is in it, and not before. A
    /// folder that will not say is not called new: that would unfreeze its
    /// naming and write our settings over its own.
    pub fn claimed(folder: &Path) -> Result<bool, String> {
        folder
            .join("rollport-library.db")
            .try_exists()
            .doing("cannot read the folder")
    }

    /// Open an existing library. This will not bring one into being: choosing
    /// a folder in a dialog makes no file in it. One that is there is brought
    /// up to this schema and version.
    pub fn open(folder: &Path) -> Result<Self, String> {
        Self::connect(folder, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
    }

    /// Open this folder's library, and make it if it is not there yet. Called
    /// when an import starts.
    pub fn create(folder: &Path) -> Result<Self, String> {
        Self::connect(
            folder,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
        )
    }

    fn connect(folder: &Path, flags: rusqlite::OpenFlags) -> Result<Self, String> {
        let db = rusqlite::Connection::open_with_flags(folder.join("rollport-library.db"), flags)
            .doing("cannot open rollport-library.db")?;

        // Another copy of this app can hold the folder too. The default is to
        // fail the moment two collide, which would abort an import that
        // already has files on disk.
        db.busy_timeout(std::time::Duration::from_secs(10))
            .doing("cannot prepare rollport-library.db")?;
        let _ = db.pragma_update(None, "journal_mode", "WAL");

        let version: i64 = db
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap_or(0);

        // The schema is the CLI's, down to the column names: its idea of what a
        // file is. The library is the app's own all the same — the CLI keeps
        // media.db, and reads this one only when pointed at it.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS media (
                 afc_path  TEXT NOT NULL,
                 st_size   INTEGER NOT NULL,
                 st_mtime  DATETIME NOT NULL,
                 synced_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                 UNIQUE(afc_path, st_size, st_mtime)
             );
             CREATE TABLE IF NOT EXISTS settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )
        .doing("cannot prepare rollport-library.db")?;

        // A CLI database renamed to this one by hand may be from before the
        // CLI's 0.2, with mtimes as text. The CLI migrates those only while
        // the version is under 2, so such a database keeps the version it has.
        // Its files are recorded again on the first run here, and the copy is
        // skipped because they are already on disk.
        let text_mtimes = version < 2
            && db
                .query_row(
                    "SELECT 1 FROM media WHERE typeof(st_mtime) = 'text' LIMIT 1",
                    [],
                    |_| Ok(()),
                )
                .is_ok();

        // Never down: a newer tool's stamp stays.
        if !text_mtimes && version < 3 {
            let _ = db.pragma_update(None, "user_version", 3);
        }

        Ok(Library(db))
    }

    /// Seconds with the fraction still on them, as the CLI writes them; the
    /// key reads whole seconds. A CLI pointed at this file compares exactly,
    /// and it rounds to the microsecond where this cuts, so it finds most of
    /// these rows but not all: the folder is this app's, not shared.
    pub fn record(&self, file: &Media) -> Result<(), String> {
        self.0
            .execute(
                "INSERT OR REPLACE INTO media (afc_path, st_size, st_mtime) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    file.path,
                    file.size as i64,
                    file.mtime.and_utc().timestamp_micros() as f64 / 1e6
                ],
            )
            .map(|_| ())
            .doing("cannot record the import")
    }

    /// An error is not a zero: zero is what unfreezes the naming.
    pub fn imported(&self) -> Result<usize, String> {
        self.0
            .query_row("SELECT count(*) FROM media", [], |row| row.get::<_, i64>(0))
            .map(|count| count as usize)
            .doing("cannot read rollport-library.db")
    }

    /// One of the folder's own answers about importing, or None where it has
    /// never been asked. Opening a folder to see what it says is not the same
    /// as answering for it: the first import writes both answers down, and
    /// until then the folder is somebody else's to read.
    ///
    /// Only a missing row is "never asked". A read that fails is not the
    /// default, or a frozen layout would name new files another way.
    pub fn setting(&self, key: &str) -> Result<Option<String>, String> {
        use rusqlite::OptionalExtension;

        self.0
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .doing("cannot read rollport-library.db")
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0
            .execute(
                "INSERT OR REPLACE INTO settings VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .map(|_| ())
            .doing("cannot write to rollport-library.db")
    }

    /// Write the folder's settings down. The layout only while no file has been
    /// named with it: the count is asked here, not of keys that may be old.
    pub fn write_settings(&self, settings: &Settings) -> Result<(), String> {
        self.set("takes", settings.takes.stored())?;

        if self.imported()? == 0 {
            self.set("layout", &settings.layout)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rollport-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(path: &str) -> Media {
        Media {
            path: path.into(),
            size: 42,
            mtime: chrono::DateTime::from_timestamp(1_700_000_000, 300_000_000)
                .unwrap()
                .naive_utc(),
            live_video: false,
        }
    }

    /// A folder the CLI filled must not look untouched. It writes that column
    /// with the fraction of a second still on it, and the key is whole seconds.
    #[test]
    fn a_row_the_cli_wrote_is_the_same_file() {
        let dir = folder("db");
        let file = file("/DCIM/100APPLE/IMG_0001.HEIC");

        let library = Library::create(&dir).unwrap();
        library
            .0
            .execute(
                "INSERT INTO media (afc_path, st_size, st_mtime) VALUES (?1, ?2, ?3)",
                rusqlite::params![file.path, file.size as i64, 1_700_000_000.3f64],
            )
            .unwrap();

        let (keys, _) = keys(&dir).unwrap();
        assert!(keys.contains(&key(&file)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The CLI looks a file up by its exact time: a row written in whole
    /// seconds would make it fetch every photo again under a _1 name.
    #[test]
    fn a_row_we_write_is_one_the_cli_can_find() {
        let dir = folder("ours");
        let library = Library::create(&dir).unwrap();
        library
            .record(&file("/DCIM/100APPLE/IMG_0001.HEIC"))
            .unwrap();

        let found: i64 = library
            .0
            .query_row(
                "SELECT count(*) FROM media WHERE st_mtime = ?1",
                [1_700_000_000.3f64],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The up-to-date screen says when the folder last took anything.
    #[test]
    fn a_library_knows_when_it_last_imported() {
        let dir = folder("last");
        let library = Library::create(&dir).unwrap();
        library
            .record(&file("/DCIM/100APPLE/IMG_0001.HEIC"))
            .unwrap();

        let (_, last) = keys(&dir).unwrap();
        let now = chrono::Utc::now().timestamp();
        assert!(last.is_some_and(|last| (now - last).abs() < 60));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A database from before the CLI's 0.2 keeps its mtimes as text. The
    /// CLI migrates it only while the version is under 2; stamping it would
    /// strand those rows for both tools.
    #[test]
    fn a_database_the_cli_has_not_migrated_keeps_its_version() {
        for (mtime, version) in [("'2023-11-14 22:13:20'", 1), ("1700000000.3", 3)] {
            let dir = folder("version");
            let db = rusqlite::Connection::open(dir.join("rollport-library.db")).unwrap();
            db.execute_batch(&format!(
                "CREATE TABLE media (afc_path TEXT NOT NULL, st_size INTEGER NOT NULL,
                     st_mtime DATETIME NOT NULL, synced_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                     UNIQUE(afc_path, st_size, st_mtime));
                 INSERT INTO media (afc_path, st_size, st_mtime)
                     VALUES ('/DCIM/100APPLE/IMG_0001.HEIC', 42, {mtime});
                 PRAGMA user_version = 1;"
            ))
            .unwrap();
            drop(db);

            let library = Library::open(&dir).unwrap();
            let stamped: i64 = library
                .0
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(stamped, version, "mtime {mtime}");

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A newer tool's stamp is not ours to lower.
    #[test]
    fn a_newer_version_stays() {
        let dir = folder("newer");
        Library::create(&dir).unwrap();
        rusqlite::Connection::open(dir.join("rollport-library.db"))
            .unwrap()
            .pragma_update(None, "user_version", 4)
            .unwrap();

        let library = Library::open(&dir).unwrap();
        let stamped: i64 = library
            .0
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(stamped, 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A library that cannot be read is not an empty one: that would fetch
    /// every file again under a _1 name.
    #[test]
    fn an_unreadable_library_is_an_error() {
        let dir = folder("garbage");
        std::fs::write(dir.join("rollport-library.db"), b"not a database at all").unwrap();

        assert!(read_folder(&dir).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reading a folder is not claiming it: no rollport-library.db is made.
    #[test]
    fn reading_a_folder_does_not_make_it_a_library() {
        let dir = folder("plain");

        let facts = read_folder(&dir).unwrap();
        assert!(facts.keys.is_empty());
        assert_eq!(facts.last, None);
        assert!(!dir.join("rollport-library.db").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Files already named with a layout keep it; what they take can change.
    #[test]
    fn a_library_with_files_keeps_its_layout() {
        let dir = folder("frozen");
        let library = Library::create(&dir).unwrap();
        let first = Settings {
            layout: "{mtime:%Y}/{name}".into(),
            takes: Takes::Photos,
        };

        library.write_settings(&first).unwrap();
        library
            .record(&file("/DCIM/100APPLE/IMG_0001.HEIC"))
            .unwrap();
        library
            .write_settings(&Settings {
                layout: default_layout(),
                takes: Takes::Videos,
            })
            .unwrap();

        let settings = read_folder(&dir).unwrap().settings;
        assert_eq!(settings.layout, first.layout);
        assert!(matches!(settings.takes, Takes::Videos));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A drive that is gone is not an empty folder: that would offer to
    /// import everything into a place that cannot take it.
    #[test]
    fn an_unplugged_drive_is_not_an_empty_folder() {
        let gone = std::env::temp_dir().join(format!("rollport-gone-{}", std::process::id()));

        assert!(read_folder(&gone).is_err());
        assert!(!gone.exists());
    }

    /// The dates picked are days here, and AFC says UTC: a photo taken late
    /// at night falls on the local day. Only seen away from UTC.
    #[test]
    fn a_range_takes_the_local_day_of_a_photo() {
        let night = ["2026-09-03 23:30:00", "2026-09-04 00:30:00"]
            .map(|at| chrono::NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M:%S").unwrap())
            .into_iter()
            .find(|utc| local(*utc).date() != utc.date());
        let Some(utc) = night else {
            return;
        };

        let photo = [Media {
            mtime: utc,
            ..file("/DCIM/100APPLE/IMG_0001.HEIC")
        }];
        let on = |day| Range::Between { from: day, to: day };
        let taken = |range| new(&photo, Takes::Everything, range, &HashSet::new()).count();

        assert_eq!(taken(on(local(utc).date())), 1);
        assert_eq!(taken(on(utc.date())), 0);
    }

    #[test]
    fn what_is_new_leaves_out_what_the_folder_already_has() {
        let media = [
            file("/DCIM/100APPLE/IMG_0001.HEIC"),
            file("/DCIM/100APPLE/IMG_0002.HEIC"),
            file("/DCIM/100APPLE/IMG_0003.HEIC"),
        ];
        let keys = HashSet::from([key(&media[1])]);

        let new: Vec<_> = new(&media, Takes::Everything, Range::Everything, &keys)
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(
            new,
            [
                "/DCIM/100APPLE/IMG_0001.HEIC",
                "/DCIM/100APPLE/IMG_0003.HEIC"
            ]
        );
    }
}
