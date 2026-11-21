//! CSB Model Optimizer: pick a game or fast-download folder, get an optimized copy (models
//! repacked or split into shared animation files, `.gz` next to every file) and a before/after
//! report. All the work is in the `csb-mdl` crate; this is the window around it.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use csb_mdl::process::{self, Options, Report};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_dialog::DialogExt;

#[derive(Clone, Serialize)]
struct Progress {
    stage: String,
    done: usize,
    total: usize,
}

#[derive(Serialize)]
struct FolderState {
    exists: bool,
    empty: bool,
    /// Looks like a game or fast-download root (has models/, maps/, sound/ or sprites/).
    game_root: bool,
}

#[tauri::command]
async fn pick_folder(app: AppHandle, start: Option<String>) -> Option<String> {
    let mut d = app.dialog().file();
    if let Some(s) = start.filter(|s| Path::new(s).is_dir()) {
        d = d.set_directory(s);
    }
    d.blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
}

#[tauri::command]
fn folder_state(path: String) -> FolderState {
    let p = Path::new(&path);
    let exists = p.is_dir();
    let empty = !exists || p.read_dir().map(|mut r| r.next().is_none()).unwrap_or(true);
    let game_root = ["models", "maps", "sound", "sprites"].iter().any(|d| p.join(d).is_dir());
    FolderState { exists, empty, game_root }
}

#[tauri::command]
fn default_output(input: String) -> String {
    let p = PathBuf::from(input.trim_end_matches(['\\', '/']));
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
    p.with_file_name(format!("{name}_optimized")).display().to_string()
}

#[tauri::command]
async fn optimize(app: AppHandle, input: String, output: String, options: Options) -> Result<Report, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let last = Mutex::new((String::new(), Instant::now()));
        let progress = |stage: &str, done: usize, total: usize| {
            let mut l = last.lock().unwrap();
            if l.0 != stage || l.1.elapsed() > Duration::from_millis(80) {
                *l = (stage.to_string(), Instant::now());
                let _ = app.emit("progress", Progress { stage: stage.to_string(), done, total });
            }
        };
        process::run(Path::new(&input), Path::new(&output), &options, &progress)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    std::process::Command::new("explorer").arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_report(path: String, report: serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![pick_folder, folder_state, default_output, optimize, open_folder, save_report])
        .run(tauri::generate_context!())
        .expect("error while running the app");
}
