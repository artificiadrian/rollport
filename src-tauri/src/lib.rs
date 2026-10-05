mod library;
mod phone;
mod state;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
};

use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, WebviewWindow,
    async_runtime::{JoinHandle, spawn, spawn_blocking},
};

use library::{Library, read_folder};
use phone::{copy_all, walk, watch};
use state::{
    End, Flow, Folder, FolderFacts, Import, Next, Range, Reading, Report, Roll, State, Takes,
};

trait Context<T> {
    fn doing(self, what: &str) -> Result<T, String>;
}

impl<T, E: std::fmt::Display> Context<T> for Result<T, E> {
    fn doing(self, what: &str) -> Result<T, String> {
        self.map_err(|e| format!("{what}: {e}"))
    }
}

/// Where we left off, the folder each phone imports into, and how large the
/// window is drawn: all the app itself remembers. Every answer about
/// importing belongs to a folder and lives in its rollport-library.db, so
/// there are no preferences here to grow.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct Remembered {
    destination: Option<PathBuf>,
    /// By `phone_key`, so the file names no phone.
    folders: HashMap<String, PathBuf>,
    zoom: Option<f64>,
}

impl Remembered {
    fn file(app: &AppHandle) -> Option<PathBuf> {
        Some(app.path().app_config_dir().ok()?.join("settings.json"))
    }

    fn load(app: &AppHandle) -> Remembered {
        Remembered::file(app)
            .and_then(|file| fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice::<Remembered>(&bytes).ok())
            .unwrap_or_default()
    }

    /// Change what is remembered. One change at a time, each read fresh: a
    /// zoom and a folder chosen at the same moment both stay.
    fn change(app: &AppHandle, change: impl FnOnce(&mut Remembered)) {
        static CHANGING: Mutex<()> = Mutex::new(());
        let _one = CHANGING.lock().unwrap_or_else(PoisonError::into_inner);

        let mut remembered = Remembered::load(app);
        change(&mut remembered);
        remembered.save(app);
    }

    /// Written beside and renamed over, so a crash halfway leaves the old
    /// file whole rather than an empty one that forgets every folder.
    fn save(&self, app: &AppHandle) {
        let Some(file) = Remembered::file(app) else {
            return;
        };

        if let Some(folder) = file.parent() {
            let _ = fs::create_dir_all(folder);
        }

        let Ok(json) = serde_json::to_vec_pretty(self) else {
            return;
        };

        let beside = file.with_extension("json.new");
        if fs::write(&beside, json).is_ok() {
            let _ = fs::rename(&beside, &file);
        }
    }
}

struct Current(Mutex<State>);

/// The state, locked. A panic under the lock must not take every later caller
/// with it: each event replaces what it changes whole, so what a poisoned lock
/// guards is still a state the window can be shown.
fn lock(app: &AppHandle) -> MutexGuard<'_, State> {
    app.state::<Current>()
        .inner()
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Change the state and show the window. The view goes out under the lock, so
/// views arrive in the order they were made. emit must not wait for the main
/// thread: do not enable tauri's `tracing` feature.
fn with<T>(app: &AppHandle, change: impl FnOnce(&mut State) -> T) -> T {
    let mut state = lock(app);
    let answer = change(&mut state);
    let _ = app.emit("flow", state.view());
    answer
}

/// Run a task; a panic becomes that task's failure.
async fn guarded<T>(task: JoinHandle<Result<T, String>>) -> Result<T, String> {
    match task.await {
        Ok(outcome) => outcome,
        Err(tauri::Error::JoinError(e)) if e.is_panic() => {
            Err(format!("stopped unexpectedly: {}", said(e.into_panic())))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// What a panic said, when it said it in words.
fn said(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

/// Open a folder the person chose: say so at once, remember it, and read it
/// without the lock.
fn choose_folder(app: &AppHandle, path: PathBuf) {
    let next = with(app, |state| {
        let opening = state.choose_destination(path);

        // The phone's folder changes even when it is the one already open.
        Remembered::change(app, |remembered| {
            if let Some(path) = &opening {
                remembered.destination = Some(path.clone());
            }
            remembered.folders = state.folders.clone();
        });

        (state.reading_needed(), opening)
    });

    follow(app, next)
}

/// Start what a change asks for: the camera roll to read, and the folder of a
/// phone just taken.
fn follow(app: &AppHandle, (reading, folder): Next) {
    if let Some(path) = folder {
        spawn(read(app.clone(), path));
    }

    read_roll(app, reading)
}

/// Read a folder, and land what it says if we are still on it.
async fn read(app: AppHandle, path: PathBuf) {
    let read = facts(&path).await;
    with(&app, |state| state.opened(&path, read))
}

/// What a folder holds, read without the lock. A panic is a broken folder,
/// not one that stays "Opening" for ever; so is a network folder whose server
/// has gone, which would otherwise hold a finished copy in Copying, refusing
/// everything.
async fn facts(path: &Path) -> Result<FolderFacts, String> {
    let path = path.to_path_buf();
    tokio::time::timeout(
        FOLDER_ANSWERS,
        guarded(spawn_blocking(move || read_folder(&path))),
    )
    .await
    .unwrap_or_else(|_| Err("the folder stopped responding".into()))
}

/// Ample for a library on a slow network drive.
const FOLDER_ANSWERS: std::time::Duration = std::time::Duration::from_secs(30);

/// Read a phone's camera roll, and again every few seconds while it fails:
/// a locked phone answers lockdown but not its own files, and unlocking it
/// says nothing to usbmuxd. It stops once nobody waits for this ticket.
fn read_roll(app: &AppHandle, reading: Option<Reading>) {
    let Some((udid, ticket)) = reading else {
        return;
    };

    let app = app.clone();

    spawn(async move {
        loop {
            let walked = guarded(spawn({
                let (app, udid) = (app.clone(), udid.clone());
                async move { walk(&app, &udid, ticket).await }
            }))
            .await;

            if !with(&app, |state| state.roll_read(ticket, walked)) {
                return;
            }

            tokio::time::sleep(LOOK_AGAIN).await;

            if lock(&app).reading(ticket).is_none() {
                return;
            }
        }
    });
}

/// How long a phone gets to be unlocked before we ask it again.
const LOOK_AGAIN: std::time::Duration = std::time::Duration::from_secs(3);

#[tauri::command]
async fn start_import(app: AppHandle) {
    let Some(Import {
        udid,
        path,
        settings,
        media,
        cancel,
    }) = with(&app, |state| {
        let import = state.start_import()?;

        Remembered::change(&app, |remembered| {
            remembered.folders = state.folders.clone()
        });

        Some(import)
    })
    else {
        return;
    };

    // Guarded, so a panic ends the run as a failure. Otherwise the run stays
    // copying for ever, everything is refused, and Stop reaches nothing.
    let run = guarded(spawn({
        let (app, path, udid) = (app.clone(), path.clone(), udid.clone());
        async move { copy_all(&app, &udid, &path, media, settings, &cancel).await }
    }))
    .await;

    // Before a file moved: the phone would not talk, or rollport-library.db would not open.
    let report = run.unwrap_or_else(|reason| Report {
        imported: 0,
        bytes: 0,
        seconds: 0,
        end: End::Failed { reason },
        trouble: None,
        newest: Vec::new(),
        udid,
    });

    // The folder is a library now, or holds more than it did, and it is the
    // one that knows how much room is left. Landed with the report, so the
    // report never shows beside the old count.
    let read = facts(&path).await;
    let next = with(&app, |state| state.copied(report, &path, read));
    follow(&app, next)
}

/// Stop the copy after the files it is on.
#[tauri::command]
async fn cancel_import(app: AppHandle) {
    lock(&app).stop()
}

#[tauri::command]
async fn flow(app: AppHandle) -> Flow {
    lock(&app).view()
}

/// Commit the pick: the highlighted row becomes the device we import from.
#[tauri::command]
async fn choose_device(udid: String, app: AppHandle) {
    let next = with(&app, |state| state.choose_device(udid));
    follow(&app, next)
}

/// A folder picked in the panel or dropped on the window. Before a folder is
/// in use it only fills the setup screen, and Use this folder takes it; once
/// one is, there is no second step, so the pick takes it at once.
fn pick(app: &AppHandle, path: PathBuf) {
    if !with(app, |state| state.offer_destination(path.clone())) {
        choose_folder(app, path)
    }
}

/// Ask for the folder to import into.
#[tauri::command]
async fn choose_destination(app: AppHandle) {
    if let Some(folder) = rfd::AsyncFileDialog::new()
        .set_title("Choose where photos go")
        .pick_folder()
        .await
    {
        pick(&app, folder.path().to_path_buf())
    }
}

/// Import into the folder on the setup screen, making it first if it is not
/// there yet (the suggested one may not be). Pressing the button is the
/// confirmation.
#[tauri::command]
async fn use_suggested_destination(app: AppHandle) {
    // Copied out first: a guard borrowed in a `let … else` lives to the
    // end of the function, and choose_folder locks again.
    let suggested = match &lock(&app).folder {
        Folder::Unset { suggested } => suggested.clone(),
        _ => None,
    };
    let Some(path) = suggested else {
        return;
    };

    // If it cannot be made, opening it says so, as for any missing folder.
    let _ = fs::create_dir_all(&path);
    choose_folder(&app, path)
}

/// The phone a file is on, if it is the phone we are on and the window was
/// told about that file: a photo the walk has announced, or once it has ended,
/// a file it found. The window asks for prints of what it was told about,
/// never for whatever it names.
fn phone_of(app: &AppHandle, udid: &str, path: &str) -> Option<String> {
    let state = lock(app);
    let phone = state.phone.as_ref().filter(|phone| phone.udid == udid)?;

    let known = match phone.roll.as_ref()? {
        Roll::Reading { announced, .. } => announced.iter().any(|file| file == path),
        Roll::Done { media } => media.iter().any(|file| file.path == path),
    };

    known.then(|| phone.udid.clone())
}

/// Serve `thumb://localhost/<udid><path on the phone>` as that file's
/// thumbnail: two phones have their own IMG_0001. A file with none answers
/// 404, and the window draws a blank print instead.
fn serve_thumbnail(
    app: &AppHandle,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    // convertFileSrc percent-encodes the whole path into one segment.
    let picture =
        percent_encoding::percent_decode_str(request.uri().path().trim_start_matches('/'))
            .decode_utf8_lossy()
            .into_owned();
    let (udid, path) = match picture.split_once('/') {
        Some((udid, rest)) => (udid, format!("/{rest}")),
        None => ("", picture.clone()),
    };
    let udid = phone_of(app, udid, &path);

    tauri::async_runtime::spawn(async move {
        let jpeg = match udid {
            Some(udid) => phone::thumbnail(&udid, &path).await.ok(),
            None => None,
        };

        let response = match jpeg {
            Some(jpeg) => tauri::http::Response::builder()
                .header(tauri::http::header::CONTENT_TYPE, "image/jpeg")
                .body(jpeg),
            None => tauri::http::Response::builder()
                .status(404)
                .body(Vec::new()),
        };

        if let Ok(response) = response {
            responder.respond(response)
        }
    });
}

/// Take a folder dragged onto the window. The panel is one way to answer the
/// question of where photos go; a drag is the other, and both end in the same
/// place. Anything that is not a folder is not an answer to it. Heard from the
/// window, not asked for by the page, so no command takes a path from it.
fn dropped(app: &AppHandle, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event
        && let Some(path) = paths.first()
        && path.is_dir()
    {
        pick(app, path.clone())
    }
}

/// Show the destination in the file manager.
#[tauri::command]
async fn reveal_destination(app: AppHandle) {
    let Some(destination) = lock(&app).folder.path().map(PathBuf::from) else {
        return;
    };

    open(destination.into_os_string())
}

/// Where the help pages are on the site.
const HELP: &str = "https://artificiadrian.github.io/rollport/help";

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Page {
    Help,
    Troubleshooting,
    #[serde(rename = "apple-devices")]
    AppleDevices,
    /// Troubleshooting, opened at "Linux does not find the iPhone".
    #[serde(rename = "linux-usbmuxd")]
    LinuxUsbmuxd,
}

/// Open a help page, or the Store page of Apple Devices, in the browser. A help
/// page is told the version, so it can speak to the one that asked.
#[tauri::command]
fn show_help(app: AppHandle, page: Page) {
    let version = &app.package_info().version;
    open(match page {
        Page::Help => format!("{HELP}?version={version}").into(),
        Page::Troubleshooting => format!("{HELP}/troubleshooting?version={version}").into(),
        // The Store page the README and the site link to.
        Page::AppleDevices => "https://apps.microsoft.com/detail/9np83lwlpz9k".into(),
        Page::LinuxUsbmuxd => {
            format!("{HELP}/troubleshooting?version={version}#linux-usbmuxd").into()
        }
    })
}

/// Hand a folder or a web address to the system, which opens it in the file
/// manager or the browser.
fn open(target: std::ffi::OsString) {
    let opener = match () {
        _ if cfg!(target_os = "macos") => "open",
        _ if cfg!(target_os = "windows") => "explorer",
        _ => "xdg-open",
    };

    // Waited for on a thread of its own: a child nobody waits for stays in
    // the process table until the app quits.
    std::thread::spawn(move || std::process::Command::new(opener).arg(target).status());
}

/// How far back to reach, for this run only.
#[tauri::command]
async fn set_range(range: Range, app: AppHandle) {
    with(&app, |state| state.set_range(range))
}

/// What this folder takes in, from now on.
#[tauri::command]
async fn set_takes(takes: Takes, app: AppHandle) {
    let changed = with(&app, |state| state.set_takes(takes));
    write_settings(&app, changed).await
}

/// Say what this folder names files, or hear why it cannot.
#[tauri::command]
async fn set_layout(layout: String, app: AppHandle) -> Result<(), &'static str> {
    let changed = with(&app, |state| state.set_layout(layout))?;
    write_settings(&app, changed).await;
    Ok(())
}

/// Write the settings the state holds into the folder's rollport-library.db, if it has
/// one; a folder that has not imported gets them with its first import. A
/// failed write is only logged: the next copy writes them again.
async fn write_settings(app: &AppHandle, path: Option<PathBuf>) {
    let Some(path) = path else {
        return;
    };

    let written = guarded(spawn_blocking({
        let (app, path) = (app.clone(), path.clone());

        move || {
            // One write at a time, each of what the state holds when it starts,
            // so an earlier answer never lands after a later one.
            static WRITING: Mutex<()> = Mutex::new(());
            let _one = WRITING.lock().unwrap_or_else(PoisonError::into_inner);

            let settings = match &lock(&app).folder {
                Folder::Known { path: on, facts } if *on == path => facts.settings.clone(),
                _ => return Ok(()),
            };

            if !Library::claimed(&path)? {
                return Ok(());
            }

            Library::open(&path)?.write_settings(&settings)
        }
    }))
    .await;

    if let Err(reason) = written {
        eprintln!("cannot save the settings of {}: {reason}", path.display())
    }
}

/// Look at the camera roll and the folder again, after a failure or a photo
/// taken since.
#[tauri::command]
async fn rescan(app: AppHandle) {
    let (reading, folder) = with(&app, |state| state.rescan());
    read_roll(&app, reading);

    if let Some(path) = folder {
        let read = facts(&path).await;
        with(&app, |state| state.reread(&path, read))
    }
}

/// The sizes ⌘+ and ⌘− step through. The layout is drawn for 1; smaller has
/// nothing to gain in a window this size.
const ZOOMS: [f64; 6] = [1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Step {
    In,
    Out,
    Actual,
}

/// Draw the page larger or smaller, and the window with it, and remember it.
fn zoom_by(app: &AppHandle, step: Step) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    // From the size shown: one the screen could not hold opens smaller than
    // the one remembered, and a step from that would change nothing.
    let now = *SHOWN.lock().unwrap_or_else(PoisonError::into_inner);
    let at = ZOOMS.iter().rposition(|zoom| *zoom <= now).unwrap_or(0);

    let next = match step {
        Step::In => ZOOMS[(at + 1).min(ZOOMS.len() - 1)],
        Step::Out => ZOOMS[at.saturating_sub(1)],
        Step::Actual => 1.0,
    };

    let zoom = zoom_to(app, &window, next);
    Remembered::change(app, |remembered| remembered.zoom = Some(zoom));
}

/// The window keeps the page's shape at every size: the page is zoomed and
/// the window grows by the same factor, so the page still fills it exactly.
/// A size the screen cannot hold is drawn at the largest step that fits.
fn zoom_to(app: &AppHandle, window: &WebviewWindow, zoom: f64) -> f64 {
    let Some((width, height)) = app
        .config()
        .app
        .windows
        .first()
        .map(|config| (config.width, config.height))
    else {
        return 1.0;
    };

    let room = window.current_monitor().ok().flatten().map(|monitor| {
        let area = monitor
            .work_area()
            .size
            .to_logical::<f64>(monitor.scale_factor());
        // The title bar is outside the page and does not grow with it.
        let bar = match (window.outer_size(), window.inner_size()) {
            (Ok(outer), Ok(inner)) => {
                f64::from(outer.height.saturating_sub(inner.height)) / monitor.scale_factor()
            }
            _ => 0.0,
        };
        ((area.height - bar) / height).min(area.width / width)
    });

    let zoom = ZOOMS
        .into_iter()
        .rfind(|step| *step <= zoom && room.is_none_or(|room| *step <= room))
        .unwrap_or(1.0);

    let _ = window.set_zoom(zoom);
    let _ = window.set_size(LogicalSize::new(width * zoom, height * zoom));
    *SHOWN.lock().unwrap_or_else(PoisonError::into_inner) = zoom;
    zoom
}

/// The zoom the window is drawn at, which the screen may have made smaller
/// than the one remembered. The remembered one stays: a larger screen
/// next time can hold it.
static SHOWN: Mutex<f64> = Mutex::new(1.0);

/// ⌘+, ⌘− and ⌘0 where there is no menu bar to hold them.
#[tauri::command]
fn zoom(app: AppHandle, step: Step) {
    zoom_by(&app, step)
}

/// The system's own menus, with the three sizes added to View where every
/// Mac app keeps them, and the help pages in Help.
#[cfg(target_os = "macos")]
fn menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let menu = Menu::default(app)?;

    let view = menu
        .items()?
        .into_iter()
        .filter_map(|item| item.as_submenu().cloned())
        .find(|submenu| submenu.text().is_ok_and(|text| text == "View"));

    if let Some(view) = view {
        view.append_items(&[
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "actual", "Actual Size", true, Some("CmdOrCtrl+0"))?,
            &MenuItem::with_id(app, "in", "Zoom In", true, Some("CmdOrCtrl+="))?,
            &MenuItem::with_id(app, "out", "Zoom Out", true, Some("CmdOrCtrl+-"))?,
        ])?;
    }

    if let Some(help) = menu.get(tauri::menu::HELP_SUBMENU_ID)
        && let Some(help) = help.as_submenu()
    {
        help.append(&MenuItem::with_id(
            app,
            "help",
            "Rollport Help",
            true,
            Some("CmdOrCtrl+Shift+/"),
        )?)?;
    }

    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let step = match event.id().as_ref() {
            "actual" => Step::Actual,
            "in" => Step::In,
            "out" => Step::Out,
            "help" => return show_help(app.clone(), Page::Help),
            _ => return,
        };
        zoom_by(app, step)
    });
    Ok(())
}

pub fn run() {
    // NVIDIA's explicit sync on Wayland crashes WebKitGTK with Error 71; off,
    // the GPU still draws. Safe: no other thread has started yet.
    #[cfg(target_os = "linux")]
    if std::path::Path::new("/proc/driver/nvidia").exists()
        && std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none()
    {
        unsafe { std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1") };
    }

    // WebKitGTK aborts on a view transition when it draws without the GPU
    // (set by hand, see the help) and in the AppImage's older copy, so the
    // page skips them there. An AppImage sets APPIMAGE.
    let no_transitions = cfg!(target_os = "linux")
        && (std::env::var_os("APPIMAGE").is_some()
            || [
                "WEBKIT_DISABLE_DMABUF_RENDERER",
                "WEBKIT_DISABLE_COMPOSITING_MODE",
            ]
            .iter()
            .any(|name| std::env::var(name).is_ok_and(|value| value != "0")));

    tauri::Builder::default()
        .append_invoke_initialization_script(format!(
            "window.__ROLLPORT_NO_TRANSITIONS__ = {no_transitions};"
        ))
        .manage(Current(Mutex::new(State::new())))
        .register_asynchronous_uri_scheme_protocol("thumb", |context, request, responder| {
            serve_thumbnail(context.app_handle(), request, responder)
        })
        .on_window_event(|window, event| dropped(window.app_handle(), event))
        .setup(|app| {
            let handle = app.handle().clone();

            #[cfg(target_os = "macos")]
            menu(&handle)?;

            // Hidden until it has its size, so it does not open small and then
            // jump.
            if let Some(window) = handle.get_webview_window("main") {
                let zoom = Remembered::load(&handle).zoom.unwrap_or(1.0);
                zoom_to(&handle, &window, zoom);
                window.show()?;
            }

            // Pictures is asked of the app, which the state is made before.
            with(&handle, |state| {
                state.folder = Folder::Unset {
                    suggested: handle
                        .path()
                        .picture_dir()
                        .ok()
                        .map(|pictures| pictures.join("iPhone")),
                }
            });

            with(&handle, |state| {
                state.folders = Remembered::load(&handle).folders;
            });

            // Opening where you left off is the same answer as choosing it.
            // The one write back it costs is the path we just read, put back
            // exactly as it was.
            if let Some(folder) = Remembered::load(&handle).destination {
                choose_folder(&handle, folder)
            }

            // The listen stream borrows its connection and is not Send, so it
            // cannot live on the shared runtime.
            std::thread::spawn(move || watch(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            flow,
            choose_device,
            choose_destination,
            use_suggested_destination,
            start_import,
            cancel_import,
            set_range,
            set_takes,
            set_layout,
            rescan,
            reveal_destination,
            show_help,
            zoom
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_panic_in_a_guarded_task_is_its_failure() {
        let failed = tauri::async_runtime::block_on(super::guarded::<()>(super::spawn(async {
            panic!("the phone went away")
        })));

        assert_eq!(
            failed,
            Err("stopped unexpectedly: the phone went away".into())
        );
    }
}
