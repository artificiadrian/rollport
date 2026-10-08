use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};

use crate::{
    library::{Key, new},
    phone::extension,
};

/// One attached device, as far as we can see it.
#[derive(Serialize, Clone, PartialEq)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Device {
    Ready {
        udid: String,
        name: String,
        version: String,
    },
    Waiting {
        udid: String,
        need: Need,
    },
}

/// What a person must do before a phone will talk to us. The window gives one
/// instruction for each, so it never guesses one from the wording of an error.
#[derive(Serialize, Clone, PartialEq)]
#[serde(
    tag = "need",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Need {
    /// The Trust prompt is on the phone and nobody has answered it.
    Trust,
    /// The phone is locked, so it cannot show the Trust prompt.
    Unlock,
    /// Trust was refused. The phone asks again only after a replug.
    Replug,
    /// Something no tap on the phone can answer.
    Failed { reason: String },
}

impl From<String> for Need {
    fn from(reason: String) -> Self {
        Need::Failed { reason }
    }
}

impl Device {
    pub fn udid(&self) -> &str {
        match self {
            Device::Ready { udid, .. } | Device::Waiting { udid, .. } => udid,
        }
    }
}

/// What usbmuxd is telling us. On Windows, missing means no Apple drivers.
pub enum Devices {
    Listed(Vec<Device>),
    Unavailable { reason: String },
}

/// Files to import, and what they are, so the window can say photos and
/// videos rather than files. A Live Photo's clip is neither: it is counted
/// apart, so the three add up to the files that will be copied.
#[derive(Serialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub files: usize,
    /// What they weigh, so the room left on the disk is answered against what
    /// will actually be fetched.
    pub bytes: u64,
    pub videos: usize,
    pub clips: usize,
    /// The newest few, by path on the phone, newest first: what the window
    /// shows as prints. A Live Photo's clip is never one; its still is.
    pub newest: Vec<String>,
}

/// How many prints the window fans out.
const PRINTS: usize = 3;

impl Batch {
    pub fn of<'a>(media: impl IntoIterator<Item = &'a Media>) -> Batch {
        let mut batch = Batch::default();
        let mut newest: Vec<&Media> = Vec::new();

        for file in media {
            batch.files += 1;
            batch.bytes += file.size;
            batch.videos += usize::from(file.is_video() && !file.is_live_video());
            batch.clips += usize::from(file.is_live_video());

            if !file.is_live_video() {
                newest.push(file);
                newest.sort_by_key(|file| std::cmp::Reverse(file.mtime));
                newest.truncate(PRINTS);
            }
        }

        batch.newest = newest.into_iter().map(|file| file.path.clone()).collect();
        batch
    }
}

/// One file on the phone, as the scan found it.
#[derive(Clone)]
pub struct Media {
    pub path: String,
    pub size: u64,
    pub mtime: chrono::NaiveDateTime,
    /// A Live Photo's clip: when its still was taken. The two halves rarely
    /// share a second, and a pair is only a pair to other apps while the
    /// names match, so the clip is named and dated with its still.
    pub still: Option<chrono::NaiveDateTime>,
}

impl Media {
    pub fn is_live_video(&self) -> bool {
        self.still.is_some()
    }

    /// The time the file is named and dated with.
    pub fn taken(&self) -> chrono::NaiveDateTime {
        self.still.unwrap_or(self.mtime)
    }

    /// A film, or a Live Photo's clip.
    pub fn is_video(&self) -> bool {
        matches!(extension(&self.path).as_deref(), Some("mov" | "mp4"))
    }
}

/// The phone we are on, and its camera roll. Choosing another phone, or this
/// one leaving, takes the roll with it.
pub struct Phone {
    pub udid: String,
    /// None until the phone is ready and a reading starts.
    pub roll: Option<Roll>,
}

pub enum Roll {
    Reading {
        /// Which reading this is. A walk that has been left behind can still
        /// report before it notices, about a question we have stopped asking.
        ticket: u64,
        seen: usize,
        /// Every photo announced so far: the only ones whose thumbnails may
        /// be served before the walk ends. The last is the one just found,
        /// which the window deals onto its prints while it waits for the count.
        announced: Vec<String>,
        /// Why the last walk failed. It is tried again until one gets through.
        failed: Option<String>,
    },
    /// Everything worth importing. The window is sent the tally, not the list.
    Done { media: Vec<Media> },
}

/// What a copy needs, taken under the lock when it starts.
pub struct Import {
    pub udid: String,
    pub path: PathBuf,
    pub settings: Settings,
    pub media: Vec<Media>,
    pub cancel: Arc<AtomicBool>,
}

/// A reading to start: which phone, and the ticket its answers carry.
pub type Reading = (String, u64);

/// What to start after a change: the camera roll to read, and the folder to read.
pub type Next = (Option<Reading>, Option<PathBuf>);

/// A phone as the app remembers it: a hash of its udid, so the file the app
/// keeps names no phone.
pub fn phone_key(udid: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(format!("rollport phone {udid}"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One counter for the whole process, so a ticket is never handed out twice.
static TICKETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What a folder names files with until someone says otherwise.
pub fn default_layout() -> String {
    "{mtime:%Y-%m-%d_%H-%M-%S}_{name}".to_string()
}

/// AFC reports a UTC instant. A file taken at 20:57 in Berlin should be called
/// 20-57, not 19-57, so names are printed in this machine's timezone.
pub fn local(mtime: chrono::NaiveDateTime) -> chrono::NaiveDateTime {
    use chrono::TimeZone;

    chrono::Local.from_utc_datetime(&mtime).naive_local()
}

/// A date, printed by a format string we did not write. chrono panics rather
/// than erring on a specifier it cannot parse, and this runs inside the lock
/// that every other thread needs, so a bad layout would take the app with it.
fn stamp(mtime: chrono::NaiveDateTime, format: &str) -> String {
    use chrono::format::{Item, StrftimeItems};
    use std::fmt::Write;

    let items: Vec<Item> = StrftimeItems::new(format).collect();

    if items.iter().any(|item| matches!(item, Item::Error)) {
        return String::new();
    }

    // %z and its kin parse cleanly and then ask a naive time for an offset it
    // does not have. chrono answers with a formatting error, and `to_string`
    // turns that into a panic, so the printing is done where the error can be
    // caught rather than where it can only be raised.
    let mut out = String::new();

    match write!(out, "{}", mtime.format_with_items(items.into_iter())) {
        Ok(()) => out,
        Err(_) => String::new(),
    }
}

pub fn render(layout: &str, name: &str, mtime: chrono::NaiveDateTime) -> String {
    let mut out = String::new();
    let mut rest = layout;

    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);

        let Some(close) = rest[open..].find('}') else {
            break;
        };

        match rest[open + 1..open + close].split_once(':') {
            Some(("mtime", format)) => out.push_str(&stamp(mtime, format)),
            None if &rest[open + 1..open + close] == "name" => out.push_str(name),
            _ => {}
        }

        rest = &rest[open + close + 1..];
    }

    out.push_str(rest);
    out
}

/// How far back this run reaches. `Last` keeps counting from today, so "the
/// last 30 days" still means that after midnight; `Between` is two fixed days,
/// because a range you typed should not move under you.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(
    tag = "range",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Range {
    Everything,
    Last {
        days: u32,
    },
    Between {
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    },
}

impl Range {
    pub fn takes(&self, day: chrono::NaiveDate) -> bool {
        match self {
            Range::Everything => true,
            // Seven days ending today is today and the six before it. Counting
            // back a full seven from today would take eight.
            // Subtracting a span from a date panics on overflow rather than
            // erring, and this runs inside the lock every other thread needs.
            // A range that reaches past the start of time reaches everything.
            Range::Last { days } => {
                let back = chrono::Duration::days(days.saturating_sub(1).into());

                chrono::Local::now()
                    .date_naive()
                    .checked_sub_signed(back)
                    .is_none_or(|first| day >= first)
            }
            Range::Between { from, to } => day >= *from && day <= *to,
        }
    }
}

/// What a folder takes in. Leaving the video half of a Live Photo behind and
/// taking no videos at all were once two answers, and their combinations were
/// never all legal — "photos only, but bring the Live Photo videos" cannot mean
/// anything. So they are one answer.
///
/// It travels with the folder, because it decides what ends up in there and it
/// means the same thing every time it is read. A date range does not: "the last
/// seven days" is a different week tomorrow, which is why that one is never
/// written down.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Takes {
    Everything,
    /// A Live Photo's .mov is part of a picture rather than a film of its own,
    /// and it is most of what a camera roll weighs.
    NoLiveVideos,
    Photos,
    Videos,
}

impl Takes {
    pub fn takes(&self, file: &Media) -> bool {
        match self {
            Takes::Everything => true,
            Takes::NoLiveVideos => !file.is_live_video(),
            Takes::Photos => !file.is_video(),
            Takes::Videos => file.is_video() && !file.is_live_video(),
        }
    }

    /// How the folder writes it down, and how it reads it back.
    pub fn stored(&self) -> &'static str {
        match self {
            Takes::Everything => "everything",
            Takes::NoLiveVideos => "noLiveVideos",
            Takes::Photos => "photos",
            Takes::Videos => "videos",
        }
    }

    /// A folder that has never said, and one that says something we do not
    /// know, both take everything: that is what a new folder does.
    pub fn read(stored: &str) -> Takes {
        [
            Takes::Everything,
            Takes::NoLiveVideos,
            Takes::Photos,
            Takes::Videos,
        ]
        .into_iter()
        .find(|takes| takes.stored() == stored)
        .unwrap_or(Takes::Everything)
    }
}

/// A path as it should be read: `~/Pictures/Camera`, not the whole thing.
/// Git Bash sets HOME to somewhere else on Windows, so the home is asked of
/// the system, and the separator is the system's.
pub fn shorten(path: &std::path::Path) -> String {
    match std::env::home_dir()
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => std::path::Path::new("~").join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

/// A name the folder can hold: something, and something under the folder
/// itself. `/Users/me/{name}` and `../{name}` would put photos where the folder
/// cannot account for them.
///
/// And a name Windows can hold, on every machine: the folder keeps its layout
/// when it moves, so `15:04_IMG_0001.HEIC` named on a Mac is a folder Windows
/// cannot sync into. These are the CLI's rules.
pub fn check_name(rendered: &str) -> Result<(), &'static str> {
    use std::path::{Component, Path};

    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    let portable = |part: &str| {
        let stem = part.split('.').next().unwrap_or_default();

        !part.ends_with([' ', '.'])
            && !part.chars().any(|c| c < ' ' || r#"<>:"|?*\"#.contains(c))
            && !RESERVED.contains(&stem.to_ascii_uppercase().as_str())
    };

    // Asked first: `..` is also a name Windows cannot hold, and "out of the
    // folder" is the answer that says what is wrong with it.
    if rendered.trim().is_empty()
        || rendered.ends_with('/')
        || !Path::new(rendered)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("The pattern must name a file inside this folder.");
    }

    if !rendered.split('/').all(portable) {
        return Err(
            "This name would not work on Windows. Leave out < > : \" | ? * \\ and names like CON.",
        );
    }

    Ok(())
}

/// The same question asked of a layout before any file is named with it, plus
/// the two the CLI learned to ask. A pattern without {name} gives every file
/// the same name, and nine thousand photos land as _1 to _9000. A date format
/// that prints nothing is a token that vanishes without saying so. The answer
/// is the first reason that applies, in words the window shows as they are.
pub fn check_layout(layout: &str) -> Result<(), &'static str> {
    // Asked of what the layout prints, not of what it says: `{{name}}` holds
    // the four letters and prints none of them.
    let printed = example(layout);

    if !printed.contains("IMG_0001.HEIC") {
        return Err("Add {name}. Without it, every file gets the same name.");
    }

    // Anything else between braces prints nothing: `{mtime}_{name}` would
    // name every file `_IMG_0001.HEIC`.
    let mut rest = layout;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let token = &rest[open + 1..open + close];
        if token != "name" && !token.starts_with("mtime:") {
            return Err("Only {name} and {mtime:…} work between braces.");
        }
        rest = &rest[open + close + 1..];
    }

    let dates_print = layout.match_indices("{mtime:").all(|(at, opener)| {
        let format = at + opener.len();

        layout[format..]
            .find('}')
            .is_some_and(|close| !stamp(specimen(), &layout[format..format + close]).is_empty())
    });

    if !dates_print {
        return Err("A {mtime:…} in this pattern gives no date. Use codes like %Y, %m and %d.");
    }

    check_name(&printed)
}

/// The instant a layout is spelled against when there is no file to spell.
fn specimen() -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2026, 9, 2)
        .and_then(|day| day.and_hms_opt(15, 4, 11))
        .unwrap_or_default()
}

/// The same layout, shown against a file that does not exist.
pub fn example(layout: &str) -> String {
    render(layout, "IMG_0001.HEIC", specimen())
}

/// Enough to answer "how long is this going to take".
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub bytes: u64,
    pub total_bytes: u64,
    /// Seconds left, at a rate that leans on the recent past. None until a
    /// rate is worth quoting.
    pub eta: Option<u64>,
    /// The file that landed last, by path on the phone: what the window deals
    /// onto its prints, at its own pace. Never a Live Photo's clip.
    pub latest: Option<String>,
}

/// What could not be copied. A count with nothing to name is a number the
/// user can do nothing with, so the two are one thing or neither.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Trouble {
    /// How many files were left on the phone.
    pub failed: usize,
    /// The first of them, by its own name, and what it said. Empty when a
    /// worker crashed before it could say which file it was on.
    pub name: String,
    pub reason: String,
}

/// The import while it runs, then what it did. Stop is the only action a
/// running copy takes, through its cancel flag.
#[derive(Serialize, Clone)]
#[serde(tag = "run", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Run {
    Copying {
        #[serde(flatten)]
        progress: Progress,
        #[serde(skip)]
        cancel: Arc<AtomicBool>,
    },
    Finished(Report),
}

/// What a run did, and how it ended.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub imported: usize,
    /// What was actually fetched, and how long it took to fetch it.
    pub bytes: u64,
    pub seconds: u64,
    #[serde(flatten)]
    pub end: End,
    /// One bad file does not end a run; it is reported at the end of it.
    pub trouble: Option<Trouble>,
    /// The newest few it copied, by path on the phone: the report's prints.
    pub newest: Vec<String>,
    /// The phone it copied from. Its Continue is only for that phone, and its
    /// prints are that phone's.
    pub udid: String,
}

#[derive(Serialize, Clone)]
#[serde(tag = "end", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum End {
    Done,
    Stopped,
    /// Stopped before the next file would leave the disk too full.
    Full,
    /// The phone left during the copy.
    Unplugged,
    Failed {
        reason: String,
    },
}

/// What a folder says about itself, read out of its own rollport-library.db. These are the
/// folder's answers, so they travel with it.
#[derive(Clone)]
pub struct FolderFacts {
    /// Every file it has taken. Empty for a folder that is not a library.
    pub keys: HashSet<Key>,
    /// When it last took anything, in seconds. None until it has.
    pub last: Option<i64>,
    /// Room an import may use on the disk the folder sits on.
    pub free: u64,
    /// The folder is on a network drive.
    pub network: bool,
    pub settings: Settings,
}

/// The folder's own answers about importing: what the person set, and
/// rollport-library.db's once it has imported.
#[derive(Clone)]
pub struct Settings {
    pub layout: String,
    pub takes: Takes,
}

/// What a new library starts with.
impl Default for Settings {
    fn default() -> Settings {
        Settings {
            layout: default_layout(),
            takes: Takes::Everything,
        }
    }
}

/// Where photos go, and how far we have got with it. A folder's own answers
/// cannot be spelled before it has given them, so they live inside the one
/// variant that has them rather than being defaulted everywhere else.
pub enum Folder {
    /// The folder offered and not yet taken: `~/Pictures/iPhone`, where the
    /// system has a Pictures folder, or one picked or dropped since.
    Unset {
        suggested: Option<PathBuf>,
    },
    /// Chosen, and being read.
    Opening(PathBuf),
    /// Chosen, and unusable. We say so rather than calling it empty.
    Broken {
        path: PathBuf,
        reason: String,
    },
    Known {
        path: PathBuf,
        facts: FolderFacts,
    },
}

impl Folder {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Folder::Unset { .. } => None,
            Folder::Opening(path) | Folder::Broken { path, .. } | Folder::Known { path, .. } => {
                Some(path)
            }
        }
    }

    pub fn facts(&self) -> Option<&FolderFacts> {
        match self {
            Folder::Known { facts, .. } => Some(facts),
            _ => None,
        }
    }
}

/// The same thing, as the window is shown it.
#[derive(Serialize, Clone)]
#[serde(
    tag = "folder",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FolderView {
    Unset {
        suggested: Option<String>,
    },
    Opening,
    Broken {
        path: String,
        reason: String,
    },
    Known {
        path: String,
        /// Files here were named with the layout, so it cannot change.
        frozen: bool,
        last_import: Option<i64>,
        free: u64,
        network: bool,
        /// What it names files with, and that layout spelled against a file.
        layout: String,
        naming: String,
        takes: Takes,
    },
}

/// The chosen phone's camera roll, as the window is shown it.
#[derive(Serialize, Clone)]
#[serde(
    tag = "roll",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RollView {
    Reading {
        seen: usize,
        /// The photo just found, by path on the phone.
        latest: Option<String>,
        failed: Option<String>,
    },
    Done {
        /// What the folder takes and the range let through. Its newest are
        /// the prints of a screen with nothing new.
        all: Batch,
        /// Those of them the folder has never taken.
        new: Batch,
    },
}

/// Where the photos come from. It changes on its own — phones attach while you
/// are choosing a folder — so it is one field of the view, not the shape of it.
#[derive(Serialize, Clone)]
#[serde(
    tag = "source",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Source {
    /// usbmuxd is not answering. On Windows, that means no Apple drivers.
    Unavailable { reason: String },
    /// Nothing is plugged in.
    Missing,
    Listed {
        /// Every attached device, for the source menu.
        devices: Vec<Device>,
        /// The one the window is about. None while a copy winds down or a
        /// report waits for a phone that is not among them.
        chosen: Option<Device>,
    },
}

/// What the window is shown, derived from the state. The window works out
/// what to show from these facts in `screen(flow)`.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    pub source: Source,
    pub folder: FolderView,
    pub roll: Option<RollView>,
    pub run: Option<Run>,
    /// The one argument to this run, never written down.
    pub range: Range,
}

pub struct State {
    /// What usbmuxd says is plugged in. Only the watcher writes it.
    pub devices: Devices,
    /// Always a listed phone: one that leaves is dropped.
    pub phone: Option<Phone>,
    pub run: Option<Run>,
    pub folder: Folder,
    pub range: Range,
    /// The folder each phone imports into, by `phone_key`. Taking a phone
    /// opens its folder; one never seen is offered the folder in use.
    pub folders: HashMap<String, PathBuf>,
}

impl State {
    /// Nothing plugged in, nothing chosen, nothing counted.
    pub fn new() -> State {
        State {
            devices: Devices::Listed(Vec::new()),
            phone: None,
            run: None,
            folder: Folder::Unset { suggested: None },
            range: Range::Everything,
            folders: HashMap::new(),
        }
    }

    /// Offer a picked or dropped folder, not yet taken: Use this folder takes
    /// it. False once a folder is in use; then a pick takes it at once.
    pub fn offer_destination(&mut self, path: PathBuf) -> bool {
        match &mut self.folder {
            Folder::Unset { suggested } => {
                *suggested = Some(path);
                true
            }
            _ => false,
        }
    }

    /// Where photos go. Answers the folder to read, if it must be read: one we
    /// already know needs no second read, and one that broke may be fixed.
    /// With a phone in use, the folder becomes that phone's. With none — a
    /// report that waited for its phone, put away by this choice — a ready
    /// phone is taken, and the folder is its.
    pub fn choose_destination(&mut self, path: PathBuf) -> Option<PathBuf> {
        if !self.act() {
            return None;
        }

        // Its own folder is set aside: this one was chosen.
        self.take_ready();

        if let Some(phone) = &self.phone {
            self.folders.insert(phone_key(&phone.udid), path.clone());
        }

        if matches!(&self.folder, Folder::Known { path: on, .. } if *on == path) {
            return None;
        }

        self.folder = Folder::Opening(path.clone());
        Some(path)
    }

    /// A read of the folder ended. An answer about a folder we have left says
    /// nothing about the one we are on.
    pub fn opened(&mut self, path: &Path, read: Result<FolderFacts, String>) {
        // Only on the folder being opened: a second read of the same folder,
        // landing late, would put back the settings changed since.
        if !matches!(&self.folder, Folder::Opening(on) if on == path) {
            return;
        }

        let path = path.to_path_buf();

        self.folder = match read {
            Ok(facts) => Folder::Known { path, facts },
            Err(reason) => Folder::Broken { path, reason },
        }
    }

    /// How far back to reach, for this run only.
    pub fn set_range(&mut self, range: Range) {
        if self.act() {
            self.range = range
        }
    }

    /// What the folder names files. Answers the folder whose rollport-library.db to
    /// write, if it changed, or why the pattern is refused.
    pub fn set_layout(&mut self, layout: String) -> Result<Option<PathBuf>, &'static str> {
        if !self.act() {
            return Ok(None);
        }

        check_layout(&layout)?;

        let Folder::Known { path, facts } = &mut self.folder else {
            return Ok(None);
        };

        // The files here were named with the layout the folder has.
        if !facts.keys.is_empty() {
            return Err("Files here are already named this way.");
        }

        facts.settings.layout = layout;
        Ok(Some(path.clone()))
    }

    /// What the folder takes in, from now on. Answers the folder whose
    /// rollport-library.db to write.
    pub fn set_takes(&mut self, takes: Takes) -> Option<PathBuf> {
        if !self.act() {
            return None;
        }

        let Folder::Known { path, facts } = &mut self.folder else {
            return None;
        };

        facts.settings.takes = takes;
        Some(path.clone())
    }

    /// What usbmuxd said. A phone that has left takes its camera roll with it,
    /// so plugging it back in reads it again instead of showing the old
    /// answer, or the old failure, for ever.
    pub fn devices(&mut self, devices: Devices) -> Next {
        self.devices = devices;

        if self
            .phone
            .as_ref()
            .is_some_and(|phone| self.listed(&phone.udid).is_none())
        {
            // A copy is always of this phone, so it has just left with four
            // workers still reading it. Each would spend three goes and a
            // timeout on every file that is left: an hour of a progress bar
            // that cannot finish.
            self.stop();
            self.phone = None;
        }

        let folder = self.take_ready();
        (self.reading_needed(), folder)
    }

    /// Commit the pick. A phone that is not listed, or is already the one we
    /// are on, changes nothing.
    pub fn choose_device(&mut self, udid: String) -> Next {
        if !self.act() || self.listed(&udid).is_none() || self.on(&udid) {
            return (None, None);
        }

        let folder = self.take(udid);
        (self.reading_needed(), folder)
    }

    /// From now on we are on this phone, and on its folder. A phone never
    /// imported is offered the folder in use, unless no phone has a folder
    /// yet: then the folder in use is everybody's, as before phones had one.
    /// Answers the folder to read.
    fn take(&mut self, udid: String) -> Option<PathBuf> {
        let known = self.folders.get(&phone_key(&udid)).cloned();
        self.phone = Some(Phone { udid, roll: None });

        match known {
            Some(path) if self.folder.path() == Some(path.as_path()) => None,
            Some(path) => {
                self.folder = Folder::Opening(path.clone());
                Some(path)
            }
            None if self.folders.is_empty() => None,
            None => {
                if let Some(path) = self.folder.path() {
                    self.folder = Folder::Unset {
                        suggested: Some(path.to_path_buf()),
                    }
                }
                None
            }
        }
    }

    /// With no phone, take a ready one, the report's phone first, but never
    /// during a copy: a copy that is winding down after its phone left still
    /// holds the only handle that can stop it. Answers the folder to read.
    fn take_ready(&mut self) -> Option<PathBuf> {
        if self.phone.is_some() || self.copying() {
            return None;
        }

        let Devices::Listed(devices) = &self.devices else {
            return None;
        };

        let reported = match &self.run {
            Some(Run::Finished(report)) => Some(report.udid.as_str()),
            _ => None,
        };
        let ready = devices
            .iter()
            .filter(|device| matches!(device, Device::Ready { .. }));
        let udid = ready
            .clone()
            .find(|device| Some(device.udid()) == reported)
            // A report that left some waits for its phone: another phone
            // would put it away unseen, so that one waits to be chosen.
            .or_else(|| self.awaited().is_none().then(|| ready.clone().next())?)?
            .udid()
            .to_string();

        // A report of another phone is over: its count and its Continue
        // would be about this one.
        if reported.is_some_and(|reported| reported != udid) {
            self.run = None
        }

        self.take(udid)
    }

    /// Read the camera roll again, after a failure or a photo taken since,
    /// and the folder, which another tool may have imported into. Answers the
    /// reading to start and the folder to read.
    pub fn rescan(&mut self) -> Next {
        if !self.act() {
            return (None, None);
        }

        if let Some(phone) = &mut self.phone {
            phone.roll = None
        }

        let folder = self.take_ready().or_else(|| match &self.folder {
            Folder::Known { path, .. } => Some(path.clone()),
            _ => None,
        });

        (self.reading_needed(), folder)
    }

    /// The reading to start, if one is due: the phone we are on is ready and
    /// its camera roll is not read or being read.
    pub fn reading_needed(&mut self) -> Option<Reading> {
        let udid = self.phone.as_ref()?.udid.clone();

        if !matches!(self.listed(&udid), Some(Device::Ready { .. })) {
            return None;
        }

        let phone = self.phone.as_mut()?;

        if phone.roll.is_some() {
            return None;
        }

        let ticket = TICKETS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        phone.roll = Some(Roll::Reading {
            ticket,
            seen: 0,
            announced: Vec::new(),
            failed: None,
        });

        Some((udid, ticket))
    }

    /// A hundred more files seen. Answers whether the walk should go on.
    pub fn roll_seen(&mut self, ticket: u64, count: usize, latest: Option<String>) -> bool {
        let Some(Roll::Reading {
            seen,
            announced,
            failed,
            ..
        }) = self.reading(ticket)
        else {
            return false;
        };

        *seen = count;
        *failed = None;

        // A hundred files with no new photo still name the same one again.
        if let Some(path) = latest
            && announced.last() != Some(&path)
        {
            announced.push(path)
        }

        true
    }

    /// A walk ended. Answers whether to try again.
    pub fn roll_read(&mut self, ticket: u64, result: Result<Vec<Media>, String>) -> bool {
        let Some(roll) = self.reading(ticket) else {
            return false;
        };

        match result {
            Ok(media) => {
                *roll = Roll::Done { media };

                // The window is about the next run now, unless the report is
                // of this phone and left some: then its Continue is that run.
                // Never the copy that is running: dropping it drops the only
                // handle that can stop it.
                if !self.copying() && !self.resumable() {
                    self.run = None
                }

                false
            }
            Err(reason) => {
                if let Roll::Reading { failed, .. } = roll {
                    *failed = Some(reason)
                }

                true
            }
        }
    }

    /// The report is of the phone we are on, and that run did not do all of it.
    fn resumable(&self) -> bool {
        self.awaited().is_some_and(|udid| self.on(udid))
    }

    /// The phone of a report whose run did not do all of it: its Continue
    /// is the next run, once that phone is read again. Not while the folder
    /// cannot be used: then the window shows the folder step, not the report.
    fn awaited(&self) -> Option<&str> {
        if !matches!(self.folder, Folder::Known { .. }) {
            return None;
        }

        match &self.run {
            Some(Run::Finished(report))
                if !matches!(report.end, End::Done) || report.trouble.is_some() =>
            {
                Some(&report.udid)
            }
            _ => None,
        }
    }

    /// The roll, while it is being read with this ticket.
    pub fn reading(&mut self, ticket: u64) -> Option<&mut Roll> {
        self.phone
            .as_mut()?
            .roll
            .as_mut()
            .filter(|roll| matches!(roll, Roll::Reading { ticket: t, .. } if *t == ticket))
    }

    fn listed(&self, udid: &str) -> Option<&Device> {
        match &self.devices {
            Devices::Listed(devices) => devices.iter().find(|device| device.udid() == udid),
            Devices::Unavailable { .. } => None,
        }
    }

    fn on(&self, udid: &str) -> bool {
        self.phone.as_ref().is_some_and(|phone| phone.udid == udid)
    }

    /// The device the window is about: our phone, or else the first listed.
    /// With no phone, a copy winding down and a report waiting for its phone
    /// are about that phone, and show no other.
    fn shown(&self) -> Option<&Device> {
        if let Some(phone) = &self.phone {
            return self.listed(&phone.udid);
        }

        if self.copying() {
            return None;
        }

        if let Some(udid) = self.awaited() {
            return self.listed(udid);
        }

        match &self.devices {
            Devices::Listed(devices) => devices.first(),
            Devices::Unavailable { .. } => None,
        }
    }

    /// Every action of the person except Stop. Refused while a copy runs: it
    /// owns the phone, the folder and the settings it started with, and its
    /// cancel handle is the only way to stop it. Otherwise a finished report is
    /// put away: the question has changed.
    fn act(&mut self) -> bool {
        if self.copying() {
            return false;
        }

        self.run = None;
        true
    }

    /// Start a copy of what is new. Answers what the copy needs: the phone,
    /// the folder with its settings, and the list the button promised.
    pub fn start_import(&mut self) -> Option<Import> {
        if !self.act() {
            return None;
        }

        let (
            Some(Device::Ready { udid, .. }),
            Some(Roll::Done { media }),
            Folder::Known { path, facts },
        ) = (
            self.phone
                .as_ref()
                .and_then(|phone| self.listed(&phone.udid)),
            self.phone.as_ref().and_then(|phone| phone.roll.as_ref()),
            &self.folder,
        )
        else {
            return None;
        };

        let media: Vec<Media> = new(media, facts.settings.takes, self.range, &facts.keys)
            .cloned()
            .collect();

        if media.is_empty() {
            return None;
        }

        let import = Import {
            udid: udid.clone(),
            path: path.clone(),
            settings: facts.settings.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            media,
        };

        // The phone imports into this folder from now on.
        self.folders
            .insert(phone_key(&import.udid), import.path.clone());

        self.run = Some(Run::Copying {
            progress: Progress {
                done: 0,
                total: import.media.len(),
                bytes: 0,
                total_bytes: import.media.iter().map(|file| file.size).sum(),
                eta: None,
                latest: None,
            },
            cancel: import.cancel.clone(),
        });

        Some(import)
    }

    /// How far the running copy has got.
    pub fn progress(&mut self, now: Progress) {
        if let Some(Run::Copying { progress, .. }) = &mut self.run {
            *progress = now
        }
    }

    /// The copy ended, in any way, and the folder was read after it. The
    /// report always lands. Answers the reading to start: a phone that became
    /// ready during the copy waited for it.
    pub fn copied(
        &mut self,
        report: Report,
        path: &Path,
        read: Result<FolderFacts, String>,
    ) -> Next {
        // Only a phone that left clears `phone` during a copy. The files it
        // was on failed because it left; Continue fetches them once it is back.
        let unplugged = self.phone.is_none()
            && (matches!(report.end, End::Stopped) || report.trouble.is_some())
            && !matches!(report.end, End::Full | End::Failed { .. });
        let report = if unplugged {
            Report {
                end: End::Unplugged,
                trouble: None,
                ..report
            }
        } else {
            report
        };

        self.run = Some(Run::Finished(report));
        self.reread(path, read);
        let folder = self.take_ready();
        (self.reading_needed(), folder)
    }

    /// The folder we are on, read again after a copy or Check again. It keeps
    /// the settings in memory, which a folder not yet imported into does not
    /// hold, and takes the rest.
    pub fn reread(&mut self, path: &Path, read: Result<FolderFacts, String>) {
        // Check again took a phone, and with it the folder it imports into.
        if matches!(&self.folder, Folder::Opening(on) if on == path) {
            return self.opened(path, read);
        }

        let Folder::Known { path: on, facts } = &mut self.folder else {
            return;
        };

        if on != path {
            return;
        }

        match read {
            Ok(read) => {
                *facts = FolderFacts {
                    settings: facts.settings.clone(),
                    ..read
                }
            }
            Err(reason) => {
                self.folder = Folder::Broken {
                    path: path.to_path_buf(),
                    reason,
                }
            }
        }
    }

    /// Tell a running copy to stop after the chunk it is on.
    pub fn stop(&self) {
        if let Some(Run::Copying { cancel, .. }) = &self.run {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed)
        }
    }

    pub fn copying(&self) -> bool {
        matches!(self.run, Some(Run::Copying { .. }))
    }

    pub fn view(&self) -> Flow {
        let source = match &self.devices {
            Devices::Unavailable { reason } => Source::Unavailable {
                reason: reason.clone(),
            },
            Devices::Listed(devices) if devices.is_empty() => Source::Missing,
            Devices::Listed(devices) => Source::Listed {
                devices: devices.clone(),
                chosen: self.shown().cloned(),
            },
        };

        // Without a folder, everything the phone has is new.
        let none = HashSet::new();
        let (takes, keys) = match self.folder.facts() {
            Some(facts) => (facts.settings.takes, &facts.keys),
            None => (Takes::Everything, &none),
        };

        let roll = self
            .phone
            .as_ref()
            .and_then(|phone| phone.roll.as_ref())
            .map(|roll| match roll {
                Roll::Reading {
                    seen,
                    announced,
                    failed,
                    ..
                } => RollView::Reading {
                    seen: *seen,
                    latest: announced.last().cloned(),
                    failed: failed.clone(),
                },
                Roll::Done { media } => RollView::Done {
                    all: Batch::of(new(media, takes, self.range, &HashSet::new())),
                    new: Batch::of(new(media, takes, self.range, keys)),
                },
            });

        Flow {
            source,
            folder: match &self.folder {
                Folder::Unset { suggested } => FolderView::Unset {
                    suggested: suggested.as_deref().map(shorten),
                },
                Folder::Opening(_) => FolderView::Opening,
                Folder::Broken { path, reason } => FolderView::Broken {
                    path: shorten(path),
                    reason: reason.clone(),
                },
                Folder::Known { path, facts } => FolderView::Known {
                    path: shorten(path),
                    frozen: !facts.keys.is_empty(),
                    last_import: facts.last,
                    free: facts.free,
                    network: facts.network,
                    layout: facts.settings.layout.clone(),
                    naming: example(&facts.settings.layout),
                    takes: facts.settings.takes,
                },
            },
            roll,
            run: self.run.clone(),
            range: self.range,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::key;

    #[test]
    fn a_batch_tells_kinds_apart_and_names_the_newest_prints() {
        let file = |path: &str, live_video: bool, second| Media {
            path: path.into(),
            size: 10,
            mtime: chrono::DateTime::from_timestamp(second, 0)
                .unwrap()
                .naive_utc(),
            still: live_video.then(chrono::NaiveDateTime::default),
        };
        let media = [
            file("/DCIM/100APPLE/IMG_0001.HEIC", false, 1),
            file("/DCIM/100APPLE/IMG_0001.MOV", true, 5),
            file("/DCIM/100APPLE/IMG_0002.MOV", false, 3),
            file("/DCIM/100APPLE/IMG_0003.JPG", false, 4),
            file("/DCIM/100APPLE/IMG_0004.JPG", false, 2),
        ];

        assert_eq!(
            Batch::of(&media),
            Batch {
                files: 5,
                bytes: 50,
                videos: 1,
                clips: 1,
                // The clip is newest of all, and is not a print.
                newest: vec![
                    "/DCIM/100APPLE/IMG_0003.JPG".into(),
                    "/DCIM/100APPLE/IMG_0002.MOV".into(),
                    "/DCIM/100APPLE/IMG_0004.JPG".into(),
                ],
            }
        );
    }

    fn when(text: &str) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn seven_days_is_today_and_the_six_before_it() {
        let range = Range::Last { days: 7 };

        // `takes` asks the clock too: a run across midnight is tried again.
        loop {
            let today = chrono::Local::now().date_naive();
            let answers = [
                range.takes(today),
                range.takes(today - chrono::Duration::days(6)),
                range.takes(today - chrono::Duration::days(7)),
            ];

            if chrono::Local::now().date_naive() == today {
                assert_eq!(answers, [true, true, false]);
                break;
            }
        }
    }

    #[test]
    fn a_layout_names_a_file() {
        let mtime = when("2026-09-02 15:04:11");

        assert_eq!(
            render("{mtime:%Y-%m-%d_%H-%M-%S}_{name}", "IMG_0001.HEIC", mtime),
            "2026-09-02_15-04-11_IMG_0001.HEIC"
        );
        assert_eq!(
            render("{mtime:%Y}/{mtime:%m}/{name}", "IMG_0001.HEIC", mtime),
            "2026/09/IMG_0001.HEIC"
        );
        assert_eq!(render("{name}", "IMG_0001.HEIC", mtime), "IMG_0001.HEIC");
    }

    #[test]
    fn a_layout_we_do_not_understand_does_not_break_the_import() {
        let mtime = when("2026-09-02 15:04:11");

        // An unknown token disappears; a brace with no closer is just text.
        assert_eq!(
            render("{nonsense}_{name}", "IMG_1.MOV", mtime),
            "_IMG_1.MOV"
        );
        assert_eq!(render("{unclosed", "IMG_1.MOV", mtime), "{unclosed");
        assert_eq!(render("", "IMG_1.MOV", mtime), "");
    }

    /// chrono panics on a specifier it cannot parse, and %z and its kin ask a
    /// naive time for an offset it has not got. Either would take the app
    /// down, since render runs under the lock every other thread needs.
    #[test]
    fn a_layout_we_cannot_print_does_not_take_the_app_down() {
        let mtime = when("2026-09-02 15:04:11");

        for bad in ["%Q", "%", "%Y%", "%E", "%z", "%:z", "%#z", "%+"] {
            assert_eq!(render(&format!("{{mtime:{bad}}}"), "IMG_1.HEIC", mtime), "");
            assert_eq!(
                check_layout(&format!("{{mtime:{bad}}}_{{name}}")),
                Err("A {mtime:…} in this pattern gives no date. Use codes like %Y, %m and %d.")
            );
        }

        assert_eq!(
            render("{mtime:%Y}-{mtime:%Q}-{name}", "IMG_1.HEIC", mtime),
            "2026--IMG_1.HEIC"
        );
    }

    #[test]
    fn a_folder_only_keeps_a_pattern_that_names_a_file_inside_it() {
        assert_eq!(check_layout(&default_layout()), Ok(()));
        assert_eq!(check_layout("{mtime:%Y}/{mtime:%m}/{name}"), Ok(()));

        // A name every file shares.
        for same in ["{nonsense}", "{{name}}", "", "{mtime:%Y}"] {
            assert_eq!(
                check_layout(same),
                Err("Add {name}. Without it, every file gets the same name."),
                "{same}"
            );
        }

        // Braces that print nothing.
        for unknown in ["{mtime}_{name}", "{nonsense}_{name}", "{name:upper}_{name}"] {
            assert_eq!(
                check_layout(unknown),
                Err("Only {name} and {mtime:…} work between braces."),
                "{unknown}"
            );
        }

        // Out of the folder, or no file at all.
        for outside in ["/Users/me/{name}", "../{name}", "{mtime:%Y}/{name}/"] {
            assert_eq!(
                check_layout(outside),
                Err("The pattern must name a file inside this folder."),
                "{outside}"
            );
        }
    }

    /// A folder moves between machines with its layout, so a name Windows
    /// cannot hold is refused on a Mac too.
    #[test]
    fn a_folder_only_keeps_a_pattern_windows_can_hold() {
        for windows in [
            "{mtime:%H:%M}_{name}",
            "{mtime:%Y}\\{name}",
            "CON/{name}",
            "nul.txt/{name}",
            "{mtime:%Y}./{name}",
            "{mtime:%Y} /{name}",
            "why?_{name}",
        ] {
            assert_eq!(
                check_layout(windows),
                Err(
                    "This name would not work on Windows. Leave out < > : \" | ? * \\ and names like CON."
                ),
                "{windows}"
            );
        }

        assert_eq!(check_layout("CONTACTS/{name}"), Ok(()));
        assert_eq!(check_layout("{mtime:%Y-%m-%d %H.%M}_{name}"), Ok(()));
    }

    #[test]
    fn a_folder_can_leave_one_half_of_the_camera_roll_behind() {
        let mut state = state(vec![phone("a")]);

        open(&mut state, folder(HashSet::new()));

        let file = |path: &str, live_video: bool| Media {
            path: path.into(),
            size: 1,
            mtime: when("2026-09-02 15:04:11"),
            still: live_video.then(|| when("2026-09-02 15:04:11")),
        };

        // A Live Photo (its still and its clip), a film, and a photo.
        read(
            &mut state,
            vec![
                file("/DCIM/100APPLE/IMG_0001.HEIC", false),
                file("/DCIM/100APPLE/IMG_0001.MOV", true),
                file("/DCIM/100APPLE/IMG_0002.MOV", false),
                file("/DCIM/100APPLE/IMG_0003.HEIC", false),
            ],
        );

        for (takes, files) in [
            (Takes::Everything, 4),
            (Takes::NoLiveVideos, 3),
            (Takes::Photos, 2),
            (Takes::Videos, 1),
        ] {
            state.set_takes(takes);
            assert_eq!(fresh(&state).files, files);
        }
        assert!(fresh(&state).newest[0].ends_with("IMG_0002.MOV"));
    }

    /// A folder that skipped Live Photo videos was answering this question
    /// before it had a name.
    #[test]
    fn a_folder_that_has_never_said_takes_everything() {
        assert!(matches!(Takes::read(""), Takes::Everything));
        assert!(matches!(Takes::read("nonsense"), Takes::Everything));
        assert!(matches!(Takes::read("photos"), Takes::Photos));
        assert!(matches!(
            Takes::read(Takes::NoLiveVideos.stored()),
            Takes::NoLiveVideos
        ));
    }

    /// Two days typed as a range are both in it.
    /// It is worked out under the lock: a panic there takes the app with it.
    #[test]
    fn the_longest_range_reaches_back_without_overflowing() {
        assert!(Range::Last { days: u32::MAX }.takes(chrono::NaiveDate::MIN));
    }

    #[test]
    fn a_range_between_two_days_takes_both() {
        let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 9, d).unwrap();
        let range = Range::Between {
            from: day(4),
            to: day(11),
        };

        assert!(range.takes(day(4)));
        assert!(range.takes(day(11)));
        assert!(!range.takes(day(3)));
        assert!(!range.takes(day(12)));
    }

    /// A run that ended changed what is new. What was new before it would
    /// offer to import what was just imported.
    #[test]
    fn a_run_that_ended_counts_again() {
        let one = media("/DCIM/100APPLE/IMG_0001.HEIC", 1);
        let two = media("/DCIM/100APPLE/IMG_0002.HEIC", 1);

        let mut state = state(vec![phone("a")]);
        open(&mut state, folder(HashSet::new()));
        read(&mut state, vec![one.clone(), two.clone()]);
        assert!(state.start_import().is_some());

        state.copied(
            finished(),
            Path::new("/tmp/x"),
            Ok(folder(HashSet::from([key(&one), key(&two)]))),
        );
        assert_eq!(fresh(&state).files, 0);
    }

    /// A folder not yet imported into holds its settings only in memory. The
    /// read says what rollport-library.db holds; the settings are the person's.
    #[test]
    fn checking_again_keeps_the_settings_the_person_set() {
        let mut state = state(vec![phone("a")]);
        open(&mut state, folder(HashSet::new()));
        state.set_takes(Takes::Photos);

        state.rescan();
        state.reread(Path::new("/tmp/x"), Ok(folder(HashSet::new())));
        assert!(matches!(
            state.view().folder,
            FolderView::Known {
                takes: Takes::Photos,
                ..
            }
        ));
    }

    fn phone(udid: &str) -> Device {
        Device::Ready {
            udid: udid.into(),
            name: "iPhone".into(),
            version: "26.1".into(),
        }
    }

    fn waiting(udid: &str, need: Need) -> Device {
        Device::Waiting {
            udid: udid.into(),
            need,
        }
    }

    /// These devices listed, as the watcher lands them.
    fn state(devices: Vec<Device>) -> State {
        let mut state = State::new();
        state.devices(Devices::Listed(devices));
        state
    }

    fn listed(devices: Vec<Device>) -> Devices {
        Devices::Listed(devices)
    }

    /// The ticket of the reading that is waited for.
    fn ticket(state: &State) -> u64 {
        match state.phone.as_ref().and_then(|phone| phone.roll.as_ref()) {
            Some(Roll::Reading { ticket, .. }) => *ticket,
            _ => panic!("no reading is waited for"),
        }
    }

    /// The reading that is waited for lands with this camera roll.
    fn read(state: &mut State, media: Vec<Media>) {
        let ticket = ticket(state);
        state.roll_read(ticket, Ok(media));
    }

    fn on(state: &State) -> Option<&str> {
        state.phone.as_ref().map(|phone| phone.udid.as_str())
    }

    /// A copy of one new file into /tmp/x, from the phone we are on.
    fn start(state: &mut State) -> Arc<AtomicBool> {
        open(state, folder(HashSet::new()));
        read(state, vec![media("/DCIM/100APPLE/IMG_0001.HEIC", 1)]);
        state.start_import().expect("the copy should start").cancel
    }

    fn finished() -> Report {
        Report {
            imported: 12,
            bytes: 1,
            seconds: 1,
            end: End::Done,
            trouble: None,
            newest: Vec::new(),
            udid: "a".into(),
        }
    }

    /// What the window is told is new.
    fn fresh(state: &State) -> Batch {
        match state.view().roll {
            Some(RollView::Done { new, .. }) => new,
            _ => panic!("the camera roll has not been read"),
        }
    }

    fn stopped(cancel: &AtomicBool) -> bool {
        cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[test]
    fn a_locked_phone_is_not_preferred_over_a_ready_one() {
        let state = state(vec![waiting("locked", Need::Unlock), phone("ready")]);

        assert_eq!(on(&state), Some("ready"));
    }

    #[test]
    fn unplugging_the_chosen_phone_falls_back_to_what_is_left() {
        let mut state = state(vec![phone("a"), phone("b")]);
        assert!(state.choose_device("b".into()).0.is_some());
        assert_eq!(on(&state), Some("b"));

        assert!(state.devices(listed(vec![phone("a")])).0.is_some());
        assert_eq!(on(&state), Some("a"));
    }

    /// Check again, or another phone: the old walk cannot land (it would
    /// show a's camera roll as b's), is told to stop at its next hundred
    /// files, and its failure is not tried again.
    #[test]
    fn a_walk_nobody_waits_for_cannot_land_and_is_told_to_stop() {
        let leaves: [fn(&mut State); 2] = [
            |state| {
                state.rescan();
            },
            |state| {
                state.choose_device("b".into());
            },
        ];

        for leave in leaves {
            let mut state = state(vec![phone("a"), phone("b")]);
            let old = ticket(&state);
            leave(&mut state);

            assert!(!state.roll_seen(old, 100, None));
            assert!(!state.roll_read(old, Err("the phone is locked".into())));
            assert!(!state.roll_read(old, Ok(Vec::new())));
            assert!(!matches!(state.view().roll, Some(RollView::Done { .. })));
            assert!(state.reading(ticket(&state)).is_some());
        }
    }

    /// While the walk runs, the photos it has announced are the only ones the
    /// window may ask thumbnails of, so every one of them is kept.
    #[test]
    fn a_walk_keeps_every_photo_it_announced() {
        let mut state = state(vec![phone("a")]);
        let ticket = ticket(&state);

        // A hundred files with no new photo name the same one again.
        for (seen, latest) in [
            (100, Some("/DCIM/1/A.HEIC")),
            (200, None),
            (300, Some("/DCIM/1/A.HEIC")),
            (400, Some("/DCIM/1/B.HEIC")),
        ] {
            assert!(state.roll_seen(ticket, seen, latest.map(String::from)));
        }

        let Some(Roll::Reading { announced, .. }) = state.reading(ticket) else {
            panic!("the walk should still be running");
        };

        assert_eq!(announced, &["/DCIM/1/A.HEIC", "/DCIM/1/B.HEIC"]);
    }

    /// The window states what the next run will do. What the last one did
    /// cannot stay in front of that once the question has changed.
    #[test]
    fn looking_again_puts_the_last_run_away() {
        let mut state = state(vec![phone("a")]);
        read(&mut state, Vec::new());
        state.run = Some(Run::Finished(finished()));

        assert!(state.rescan().0.is_some());
        assert!(state.run.is_none());
    }

    #[test]
    fn a_running_copy_is_not_replaced_behind_its_back() {
        let mut state = state(vec![phone("a"), phone("b")]);
        start(&mut state);

        assert!(state.start_import().is_none());
        assert_eq!(state.choose_device("b".into()).0, None);
        assert_eq!(state.rescan(), (None, None));
        assert_eq!(state.choose_destination("/tmp/y".into()), None);
        state.set_range(Range::Last { days: 7 });
        assert_eq!(state.set_takes(Takes::Photos), None);
        assert_eq!(state.set_layout("{name}".into()), Ok(None));

        // Every one of those used to drop the job, and with it the only handle
        // that can stop the four workers still reading the phone.
        assert!(matches!(
            &state.run,
            Some(Run::Copying {
                progress: Progress { total: 1, .. },
                ..
            })
        ));
        assert_eq!(on(&state), Some("a"));
        assert!(matches!(state.view().roll, Some(RollView::Done { .. })));
        assert!(state.range == Range::Everything);
        let settings = &state.folder.facts().expect("the folder is open").settings;
        assert!(matches!(settings.takes, Takes::Everything));
        assert_eq!(settings.layout, default_layout());
    }

    #[test]
    fn unplugging_a_phone_forgets_what_its_camera_roll_said() {
        let mut state = state(vec![phone("a")]);
        let ticket = ticket(&state);
        assert!(state.roll_read(ticket, Err("device socket io failed".into())));

        // Replugging must look again rather than show the old failure for ever.
        state.devices(listed(vec![]));
        assert!(state.phone.is_none());
        assert!(state.devices(listed(vec![phone("a")])).0.is_some());
    }

    #[test]
    fn nothing_is_importable_without_a_phone_a_folder_and_something_new() {
        let mut state = state(vec![]);
        assert!(state.start_import().is_none());

        state.devices(listed(vec![waiting("a", Need::Unlock)]));
        assert!(state.start_import().is_none());

        let file = media("/DCIM/100APPLE/IMG_0001.HEIC", 1);
        state.devices(listed(vec![phone("a")]));
        read(&mut state, vec![file.clone()]);
        assert!(state.start_import().is_none());

        open(&mut state, folder(HashSet::from([key(&file)])));
        assert!(state.start_import().is_none());

        state.reread(Path::new("/tmp/x"), Ok(folder(HashSet::new())));
        let import = state.start_import().expect("the copy should start");
        assert_eq!((import.udid.as_str(), import.media.len()), ("a", 1));
        assert!(state.copying());
    }

    /// The phone being copied has left, or usbmuxd has: four workers would
    /// spend three goes and a timeout on every file that is left.
    #[test]
    fn a_copy_stops_when_its_phone_goes() {
        for gone in [
            listed(vec![phone("b")]),
            Devices::Unavailable {
                reason: "usbmuxd is unreachable".into(),
            },
        ] {
            let mut state = state(vec![phone("a"), phone("b")]);
            let cancel = start(&mut state);

            state.devices(gone);
            assert!(stopped(&cancel));
            assert!(state.phone.is_none());
        }
    }

    #[test]
    fn a_copy_whose_phone_left_says_so() {
        let ended = |unplug: bool, report: Report| {
            let mut state = state(vec![phone("a")]);
            start(&mut state);
            if unplug {
                state.devices(listed(vec![]));
            }
            state.copied(report, Path::new("/tmp/x"), Ok(folder(HashSet::new())));
            match &state.run {
                Some(Run::Finished(report)) => (report.end.clone(), report.trouble.is_some()),
                _ => panic!("the run did not finish"),
            }
        };
        let stopped = Report {
            end: End::Stopped,
            ..finished()
        };
        let lost = Report {
            trouble: Some(Trouble {
                failed: 3,
                name: "IMG_0001.HEIC".into(),
                reason: "the device is gone".into(),
            }),
            ..finished()
        };
        let full = Report {
            end: End::Full,
            ..finished()
        };

        assert!(matches!(
            ended(true, stopped.clone()),
            (End::Unplugged, false)
        ));
        assert!(matches!(ended(true, lost.clone()), (End::Unplugged, false)));
        assert!(matches!(ended(true, finished()), (End::Done, false)));
        assert!(matches!(ended(true, full), (End::Full, false)));
        // What the folder said is the news, not the cable.
        let failed = Report {
            end: End::Failed {
                reason: "the disk is gone".into(),
            },
            ..lost.clone()
        };
        assert!(matches!(ended(true, failed), (End::Failed { .. }, true)));
        assert!(matches!(ended(false, stopped), (End::Stopped, false)));
        assert!(matches!(ended(false, lost), (End::Done, true)));
    }

    /// A copy of phone a whose cable came out, as it lands.
    fn pulled(state: &mut State, end: End) {
        start(state);
        state.devices(listed(vec![]));
        state.copied(
            Report { end, ..finished() },
            Path::new("/tmp/x"),
            Ok(folder(HashSet::new())),
        );
    }

    fn reported(state: &State) -> bool {
        matches!(state.run, Some(Run::Finished(_)))
    }

    /// Another phone would put a report that left some away unseen, so it
    /// waits to be chosen; once chosen, the report goes.
    #[test]
    fn another_phone_waits_while_a_report_waits_for_its_own() {
        let mut state = state(vec![phone("a")]);
        pulled(&mut state, End::Stopped);

        state.devices(listed(vec![phone("b")]));
        assert_eq!(on(&state), None);
        assert!(reported(&state));
        assert!(matches!(
            state.view().source,
            Source::Listed { chosen: None, .. }
        ));

        state.choose_device("b".into());
        assert_eq!(on(&state), Some("b"));
        assert!(!reported(&state));
    }

    /// A report of a run that did all of it waits for nothing.
    #[test]
    fn another_phone_puts_a_finished_report_away() {
        let mut state = state(vec![phone("a")]);
        pulled(&mut state, End::Done);

        state.devices(listed(vec![phone("b")]));
        assert_eq!(on(&state), Some("b"));
        assert!(!reported(&state));
    }

    /// The report's phone back, but locked: the window is about it, not about
    /// another phone that is ready.
    #[test]
    fn a_report_shows_its_own_phone_while_it_waits() {
        let mut state = state(vec![phone("a")]);
        pulled(&mut state, End::Stopped);

        state.devices(listed(vec![phone("b"), waiting("a", Need::Unlock)]));
        assert_eq!(on(&state), None);
        assert!(matches!(
            state.view().source,
            Source::Listed { chosen: Some(Device::Waiting { ref udid, .. }), .. } if udid == "a"
        ));
    }

    #[test]
    fn the_report_phone_is_taken_first() {
        let mut state = state(vec![phone("a")]);
        pulled(&mut state, End::Stopped);

        state.devices(listed(vec![phone("b"), phone("a")]));
        assert_eq!(on(&state), Some("a"));
        assert!(reported(&state));
    }

    /// Its Continue is the next run, once the camera roll is read again.
    #[test]
    fn a_run_that_left_some_keeps_its_report_when_its_phone_is_back() {
        let mut state = state(vec![phone("a")]);
        pulled(&mut state, End::Stopped);

        state.devices(listed(vec![waiting("a", Need::Unlock)]));
        assert!(reported(&state));
        state.devices(listed(vec![phone("a")]));
        read(&mut state, vec![media("/DCIM/100APPLE/IMG_0001.HEIC", 1)]);

        assert!(matches!(
            state.run,
            Some(Run::Finished(Report {
                end: End::Unplugged,
                ..
            }))
        ));
        assert!(state.start_import().is_some());
    }

    #[test]
    fn a_run_that_did_all_of_it_is_over_when_its_phone_is_read_again() {
        let mut state = state(vec![phone("a")]);
        pulled(&mut state, End::Done);

        state.devices(listed(vec![phone("a")]));
        assert!(reported(&state));
        read(&mut state, vec![media("/DCIM/100APPLE/IMG_0001.HEIC", 1)]);
        assert!(!reported(&state));
    }

    fn folder_of(state: &State, udid: &str) -> Option<PathBuf> {
        state.folders.get(&phone_key(udid)).cloned()
    }

    /// Choosing a folder while on a phone, or importing, pairs them.
    #[test]
    fn a_phone_keeps_the_folder_it_imports_into() {
        let mut state = state(vec![phone("a")]);
        open(&mut state, folder(HashSet::new()));
        assert_eq!(folder_of(&state, "a"), Some("/tmp/x".into()));

        state.choose_destination("/tmp/y".into());
        assert_eq!(folder_of(&state, "a"), Some("/tmp/y".into()));
    }

    #[test]
    fn taking_a_phone_opens_its_folder() {
        let mut state = state(vec![phone("a"), phone("b")]);
        open(&mut state, folder(HashSet::new()));
        state.folders.insert(phone_key("b"), "/tmp/b".into());

        assert_eq!(state.choose_device("b".into()).1, Some("/tmp/b".into()));
        assert!(matches!(&state.folder, Folder::Opening(path) if path == Path::new("/tmp/b")));

        // Back to a: its folder again.
        state.opened(Path::new("/tmp/b"), Ok(folder(HashSet::new())));
        assert_eq!(state.choose_device("a".into()).1, Some("/tmp/x".into()));
    }

    /// One click for a new iPhone that goes on with the same library; a
    /// choice for a second person's.
    #[test]
    fn a_phone_never_imported_is_offered_the_folder_in_use() {
        let mut state = state(vec![phone("a"), phone("b")]);
        open(&mut state, folder(HashSet::new()));

        assert_eq!(state.choose_device("b".into()).1, None);
        assert!(matches!(
            &state.folder,
            Folder::Unset { suggested: Some(path) } if path == Path::new("/tmp/x")
        ));
    }

    /// The folder remembered from before phones had one is everybody's.
    #[test]
    fn before_any_phone_has_a_folder_the_one_in_use_is_kept() {
        let mut state = state(vec![phone("a"), phone("b")]);
        open(&mut state, folder(HashSet::new()));
        state.folders.clear();

        assert_eq!(state.choose_device("b".into()).1, None);
        assert!(matches!(state.folder, Folder::Known { .. }));
    }

    /// Phones come and go while a copy runs: none of them takes the window,
    /// the folder or the run from it, and when it ends its own phone is
    /// taken first, with its report.
    #[test]
    fn phones_switched_during_a_copy_wait_for_its_end() {
        let mut state = state(vec![phone("a")]);
        state.folders.insert(phone_key("b"), "/tmp/b".into());
        start(&mut state);

        // b arrives, and is chosen: refused.
        assert_eq!(
            state.devices(listed(vec![phone("a"), phone("b")])),
            (None, None)
        );
        assert_eq!(state.choose_device("b".into()), (None, None));
        assert_eq!(on(&state), Some("a"));

        // a's cable comes out, and goes back in, before the copy has wound down.
        state.devices(listed(vec![phone("b")]));
        assert_eq!(
            state.devices(listed(vec![phone("b"), phone("a")])),
            (None, None)
        );
        assert_eq!(on(&state), None);
        assert!(matches!(state.folder, Folder::Known { .. }));

        let (reading, folder) = state.copied(
            Report {
                end: End::Stopped,
                ..finished()
            },
            Path::new("/tmp/x"),
            Ok(folder(HashSet::new())),
        );
        assert_eq!(reading.map(|(udid, _)| udid), Some("a".into()));
        assert_eq!(folder, None);
        assert!(matches!(
            state.run,
            Some(Run::Finished(Report {
                end: End::Unplugged,
                ..
            }))
        ));
    }

    /// Only b is left when the copy of a ends: the report waits for a, and
    /// choosing b takes b with its folder.
    #[test]
    fn a_copy_whose_phone_left_waits_for_it_beside_the_phone_that_stayed() {
        let mut state = state(vec![phone("a"), phone("b")]);
        state.folders.insert(phone_key("b"), "/tmp/b".into());
        start(&mut state);
        state.devices(listed(vec![phone("b")]));

        let (reading, folder) = state.copied(
            Report {
                end: End::Stopped,
                ..finished()
            },
            Path::new("/tmp/x"),
            Ok(folder(HashSet::new())),
        );
        assert_eq!((reading, folder), (None, None));
        assert!(reported(&state));

        let (reading, folder) = state.choose_device("b".into());
        assert_eq!(reading.map(|(udid, _)| udid), Some("b".into()));
        assert_eq!(folder, Some("/tmp/b".into()));
        assert!(state.run.is_none());
    }

    /// The same folder dropped while it opens: two reads, and the second
    /// lands after Options changed it.
    #[test]
    fn a_second_read_of_the_open_folder_keeps_what_changed_since() {
        let mut state = state(vec![phone("a")]);
        open(&mut state, folder(HashSet::new()));
        state.set_takes(Takes::Photos);

        state.opened(Path::new("/tmp/x"), Ok(folder(HashSet::new())));
        assert!(matches!(
            state.folder.facts().map(|facts| facts.settings.takes),
            Some(Takes::Photos)
        ));
    }

    /// Files lost while the phone stayed: once it is back, they are what
    /// Continue fetches.
    #[test]
    fn a_run_that_lost_files_keeps_its_continue_after_a_replug() {
        let mut state = state(vec![phone("a")]);
        start(&mut state);
        let lost = Report {
            trouble: Some(Trouble {
                failed: 1,
                name: "IMG_0001.HEIC".into(),
                reason: "the file vanished".into(),
            }),
            ..finished()
        };
        state.copied(lost, Path::new("/tmp/x"), Ok(folder(HashSet::new())));

        state.devices(listed(vec![]));
        state.devices(listed(vec![phone("a")]));
        read(&mut state, vec![media("/DCIM/100APPLE/IMG_0001.HEIC", 1)]);

        assert!(matches!(
            &state.run,
            Some(Run::Finished(Report {
                trouble: Some(_),
                ..
            }))
        ));
        assert!(state.start_import().is_some());
    }

    /// Read, then locked: the button would start a copy that cannot read.
    #[test]
    fn a_phone_that_locks_after_its_read_cannot_start_an_import() {
        let mut state = state(vec![phone("a")]);
        open(&mut state, folder(HashSet::new()));
        read(&mut state, vec![media("/DCIM/100APPLE/IMG_0001.HEIC", 1)]);

        state.devices(listed(vec![waiting("a", Need::Unlock)]));
        assert!(state.start_import().is_none());
    }

    /// The report waited for a; b is there, and a folder is chosen: the
    /// report goes, b is taken and read, and the folder is b's.
    #[test]
    fn choosing_a_folder_while_a_report_waits_takes_the_phone_there() {
        let mut state = state(vec![phone("a")]);
        pulled(&mut state, End::Stopped);
        state.devices(listed(vec![phone("b")]));
        assert_eq!(on(&state), None);

        assert_eq!(
            state.choose_destination("/tmp/y".into()),
            Some("/tmp/y".into())
        );
        assert_eq!(on(&state), Some("b"));
        assert!(!reported(&state));
        assert_eq!(folder_of(&state, "b"), Some("/tmp/y".into()));
        assert!(matches!(&state.folder, Folder::Opening(path) if path == Path::new("/tmp/y")));
        assert!(state.reading_needed().is_some());
    }

    /// The drive went with the cable: no report to wait behind, so the phone
    /// that is there is taken, and the window can ask for a folder.
    #[test]
    fn a_report_beside_a_broken_folder_waits_for_nothing() {
        let mut state = state(vec![phone("a")]);
        start(&mut state);
        state.devices(listed(vec![]));
        state.copied(
            Report {
                end: End::Stopped,
                ..finished()
            },
            Path::new("/tmp/x"),
            Err("cannot find the folder".into()),
        );

        state.devices(listed(vec![phone("b")]));
        assert_eq!(on(&state), Some("b"));
    }

    /// Two phones, the second one copying. Tapping Trust on the first must
    /// not take the window, or the unplug rule, away from the copy.
    #[test]
    fn a_copy_keeps_its_phone_when_another_becomes_ready() {
        let mut state = state(vec![waiting("a", Need::Trust), phone("b")]);
        assert_eq!(on(&state), Some("b"));
        let cancel = start(&mut state);

        assert_eq!(state.devices(listed(vec![phone("a"), phone("b")])).0, None);
        assert_eq!(on(&state), Some("b"));

        state.devices(listed(vec![phone("a")]));
        assert!(stopped(&cancel));
    }

    #[test]
    fn a_chosen_phone_is_read_once_it_is_trusted() {
        let mut state = state(vec![waiting("a", Need::Trust)]);
        assert_eq!(state.choose_device("a".into()).0, None);
        assert!(state.view().roll.is_none());

        let reading = state.devices(listed(vec![phone("a")])).0;
        assert_eq!(reading, Some(("a".into(), ticket(&state))));
    }

    /// A locked phone fails its walk until it is unlocked. The failure is
    /// shown while it lasts, and goes as soon as a walk gets through.
    #[test]
    fn a_failed_reading_is_tried_again_and_its_failure_clears() {
        let mut state = state(vec![phone("a")]);
        let ticket = ticket(&state);

        assert!(state.roll_read(ticket, Err("the phone is locked".into())));
        assert!(matches!(
            state.view().roll,
            Some(RollView::Reading {
                failed: Some(_),
                ..
            })
        ));

        assert!(state.roll_seen(ticket, 100, None));
        assert!(matches!(
            state.view().roll,
            Some(RollView::Reading {
                seen: 100,
                failed: None,
                ..
            })
        ));
    }

    /// A copy is never put away by a reading: its cancel flag is the only way
    /// to stop it. No path starts a reading during a copy today; this keeps
    /// it so if one does.
    #[test]
    fn a_reading_that_lands_during_a_copy_leaves_the_copy() {
        let mut state = state(vec![phone("a")]);
        start(&mut state);
        state.phone.as_mut().unwrap().roll = None;
        state.reading_needed();
        read(&mut state, Vec::new());
        assert!(state.copying());
    }

    /// Choosing the folder we can already see reads nothing again. One that
    /// broke is read again: the person may have fixed it.
    #[test]
    fn the_same_folder_again_does_nothing_unless_it_is_broken() {
        let mut state = State::new();
        open(&mut state, folder(HashSet::new()));
        assert_eq!(state.choose_destination("/tmp/x".into()), None);
        assert!(matches!(state.folder, Folder::Known { .. }));

        state.reread(Path::new("/tmp/x"), Err("cannot read the folder".into()));
        assert_eq!(
            state.choose_destination("/tmp/x".into()),
            Some("/tmp/x".into())
        );
        assert!(matches!(state.folder, Folder::Opening(_)));
    }

    #[test]
    fn an_open_that_lands_after_another_choice_is_dropped() {
        let mut state = State::new();
        state.choose_destination("/tmp/x".into());
        state.choose_destination("/tmp/y".into());

        state.opened(Path::new("/tmp/x"), Ok(folder(HashSet::new())));
        assert!(matches!(&state.folder, Folder::Opening(path) if path == Path::new("/tmp/y")));
    }

    /// The files already in the folder were named with its layout; a new one
    /// would name what comes next another way.
    #[test]
    fn a_layout_is_refused_once_the_folder_has_keys() {
        let mut state = State::new();
        open(&mut state, folder(HashSet::new()));
        assert_eq!(
            state.set_layout("{mtime:%Y}/{name}".into()),
            Ok(Some("/tmp/x".into()))
        );

        let mut state = State::new();
        open(
            &mut state,
            folder(HashSet::from([key(&media("/DCIM/a", 1))])),
        );
        assert_eq!(
            state.set_layout("{mtime:%Y}/{name}".into()),
            Err("Files here are already named this way.")
        );
        assert!(matches!(
            state.view().folder,
            FolderView::Known { frozen: true, .. }
        ));
        assert_eq!(
            state
                .folder
                .facts()
                .map(|facts| facts.settings.layout.clone()),
            Some(default_layout())
        );
    }

    /// A phone that became ready while another was copied waits for the end
    /// of the copy: the copy's phone is gone, and it holds the only handle.
    #[test]
    fn a_second_phone_is_taken_after_a_copy_of_the_first_ends() {
        let mut state = state(vec![phone("a"), phone("b")]);
        start(&mut state);

        assert_eq!(state.devices(listed(vec![phone("b")])).0, None);
        assert_eq!(on(&state), None);

        let ended = state.copied(finished(), Path::new("/tmp/x"), Ok(folder(HashSet::new())));
        assert!(ended.0.is_some());
        assert_eq!(on(&state), Some("b"));
    }

    /// A drive pulled out during a copy: the folder step says so.
    #[test]
    fn a_failed_read_after_a_copy_makes_the_folder_broken() {
        let mut state = state(vec![phone("a")]);
        start(&mut state);

        state.copied(
            finished(),
            Path::new("/tmp/x"),
            Err("cannot read the folder".into()),
        );
        assert!(matches!(state.folder, Folder::Broken { .. }));
    }

    fn media(path: &str, size: u64) -> Media {
        Media {
            path: path.into(),
            size,
            mtime: when("2026-09-02 15:04:11"),
            still: None,
        }
    }

    /// A plain folder at /tmp/x that has taken these files.
    fn folder(keys: HashSet<Key>) -> FolderFacts {
        FolderFacts {
            keys,
            last: None,
            free: u64::MAX,
            network: false,
            settings: Settings::default(),
        }
    }

    /// /tmp/x is chosen, and read with these facts.
    fn open(state: &mut State, facts: FolderFacts) {
        state.choose_destination("/tmp/x".into());
        state.opened(Path::new("/tmp/x"), Ok(facts));
    }
}
