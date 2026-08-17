//! Shapes of War launcher — a small Rust app players run instead of the game
//! directly. On open it checks GitHub for a newer release of `ShapesOfWar.exe`,
//! downloads it if one exists, and a Play button launches whatever is
//! installed next to the launcher.
//!
//! A port of the original Python launcher (launcher/launcher.py) to Rust, with
//! the same behaviour: unauthenticated GET of the latest release, download the
//! named asset, atomically swap it in, write a `game_version.txt` marker, and
//! fall back to the installed build if the network is down.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

const REPO: &str = "LemonMoo/ShapesOfWar-rs";
const RELEASE_API: &str = "https://api.github.com/repos/LemonMoo/ShapesOfWar-rs/releases/latest";
const GAME_EXE_NAME: &str = "ShapesOfWar.exe";
const USER_AGENT: &str = "ShapesOfWarLauncher";

const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8a, 0x94, 0xa6);
const GOOD: egui::Color32 = egui::Color32::from_rgb(0x59, 0xc1, 0x7a);
const BAD: egui::Color32 = egui::Color32::from_rgb(0xe2, 0x60, 0x4a);

/// Where the launcher's persistent files live: next to its own exe (a frozen
/// onefile exe has no meaningful `__file__`; `current_exe` is the real path).
fn app_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

enum UpdateEvent {
    UpToDate(String),
    Updated(String),
    CheckFailed,
    DownloadFailed(String),
    Progress(u64, u64),
}

/// (tag, download_url) for the latest release's `ShapesOfWar.exe`, or None on
/// any network/parse failure.
fn check_latest() -> Option<(String, String)> {
    let mut resp = ureq::get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", USER_AGENT)
        .call()
        .ok()?;
    let body = resp.body_mut().read_to_string().ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let tag = json.get("tag_name")?.as_str()?.to_string();
    for asset in json.get("assets")?.as_array()? {
        if asset.get("name")?.as_str()? == GAME_EXE_NAME {
            let url = asset.get("browser_download_url")?.as_str()?.to_string();
            return Some((tag, url));
        }
    }
    None
}

/// Stream `url` to a staging file, then swap it in as the live exe. The live
/// exe is only touched after the download fully lands.
fn download(url: &str, dest: &PathBuf, tmp: &PathBuf, tx: &Sender<UpdateEvent>) -> Result<(), String> {
    let mut resp = ureq::get(url)
        .header("Accept", "application/octet-stream")
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| e.to_string())?;
    let total: u64 = resp
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = resp.body_mut().as_reader();
    let mut file = std::fs::File::create(tmp).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 1 << 16];
    let mut done: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        let _ = tx.send(UpdateEvent::Progress(done, total));
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    // Windows `rename` won't overwrite an existing file, so remove-then-rename
    // (a tiny non-atomic gap vs the Python os.replace, acceptable for a
    // launcher — the staging file is complete before this point).
    if dest.exists() {
        let _ = std::fs::remove_file(dest);
    }
    std::fs::rename(tmp, dest).map_err(|e| e.to_string())?;
    Ok(())
}

fn read_installed_version(root: &PathBuf) -> Option<String> {
    std::fs::read_to_string(root.join("game_version.txt"))
        .ok()
        .map(|s| s.trim().to_string())
}

fn write_installed_version(root: &PathBuf, tag: &str) {
    let _ = std::fs::write(root.join("game_version.txt"), tag);
}

fn start_update_check(root: PathBuf, tx: Sender<UpdateEvent>) {
    std::thread::spawn(move || {
        let game_exe = root.join(GAME_EXE_NAME);
        let tmp = root.join(format!("{GAME_EXE_NAME}.download"));
        let result = check_latest();
        let Some((tag, url)) = result else {
            let _ = tx.send(UpdateEvent::CheckFailed);
            return;
        };
        if read_installed_version(&root).as_deref() == Some(tag.as_str()) && game_exe.exists() {
            let _ = tx.send(UpdateEvent::UpToDate(tag));
            return;
        }
        match download(&url, &game_exe, &tmp, &tx) {
            Ok(()) => {
                write_installed_version(&root, &tag);
                let _ = tx.send(UpdateEvent::Updated(tag));
            }
            Err(e) => {
                let _ = tx.send(UpdateEvent::DownloadFailed(e));
            }
        }
    });
}

struct LauncherApp {
    root: PathBuf,
    status: String,
    status_color: egui::Color32,
    play_enabled: bool,
    rx: Receiver<UpdateEvent>,
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                UpdateEvent::UpToDate(tag) => {
                    self.status = format!("Up to date ({tag}).");
                    self.status_color = GOOD;
                    self.play_enabled = true;
                }
                UpdateEvent::Updated(tag) => {
                    self.status = format!("Updated to {tag}. Ready to play.");
                    self.status_color = GOOD;
                    self.play_enabled = true;
                }
                UpdateEvent::CheckFailed => {
                    if self.root.join(GAME_EXE_NAME).exists() {
                        self.status =
                            "Couldn't check for updates — playing installed version.".into();
                        self.play_enabled = true;
                    } else {
                        self.status = "No internet connection — first install needs one.".into();
                        self.status_color = BAD;
                    }
                }
                UpdateEvent::DownloadFailed(e) => {
                    if self.root.join(GAME_EXE_NAME).exists() {
                        self.status =
                            "Update download failed — playing installed version.".into();
                        self.status_color = BAD;
                        self.play_enabled = true;
                    } else {
                        self.status = format!("Update download failed: {e}");
                        self.status_color = BAD;
                    }
                }
                UpdateEvent::Progress(done, total) => {
                    if total > 0 {
                        let pct = done * 100 / total;
                        self.status = format!("Downloading update… {pct}%");
                    } else {
                        self.status = "Downloading update…".into();
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(28.0);
                ui.heading("Shapes of War");
                ui.add_space(6.0);
                ui.colored_label(self.status_color, &self.status);
                ui.add_space(16.0);
                if ui
                    .add_enabled(
                        self.play_enabled,
                        egui::Button::new("Play").min_size(egui::vec2(140.0, 44.0)),
                    )
                    .clicked()
                {
                    self.play();
                }
            });
        });
        // Poll the worker channel even while idle, so status/progress updates
        // land without waiting for input.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl LauncherApp {
    fn play(&mut self) {
        let game = self.root.join(GAME_EXE_NAME);
        if !game.exists() {
            self.status = "Game not found — update failed or hasn't run yet.".into();
            self.status_color = BAD;
            return;
        }
        let _ = std::process::Command::new(&game)
            .current_dir(&self.root)
            .spawn();
        std::process::exit(0);
    }
}

fn main() -> eframe::Result {
    let root = app_root();
    let (tx, rx) = channel();
    start_update_check(root.clone(), tx);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 220.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "Shapes of War Launcher",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(LauncherApp {
                root,
                status: "Checking for updates…".into(),
                status_color: MUTED,
                play_enabled: false,
                rx,
            }))
        }),
    )
}
