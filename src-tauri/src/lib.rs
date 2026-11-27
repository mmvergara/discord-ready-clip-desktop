mod ffmpeg;
mod server;

use ffmpeg::{ExportResult, Job, JobHandle, Probe, SharedHandle};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    server: server::VideoServer,
    jobs: Arc<Mutex<HashMap<u32, SharedHandle>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenedVideo {
    url: String,
    name: String,
    path: String,
    #[serde(flatten)]
    probe: Probe,
}

#[tauri::command]
async fn open_video(state: State<'_, AppState>, path: String) -> Result<OpenedVideo, String> {
    let p = PathBuf::from(&path);
    let probe = {
        let p = p.clone();
        tauri::async_runtime::spawn_blocking(move || ffmpeg::probe(&p)).await.map_err(|e| e.to_string())??
    };
    Ok(OpenedVideo {
        url: state.server.register(&p),
        name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path,
        probe,
    })
}

/// Which encoder exports will use. The first call runs a quick test encode on each candidate.
#[tauri::command]
async fn encoder_name() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| ffmpeg::encoder().label.to_string()).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn export(app: AppHandle, state: State<'_, AppState>, job: Job) -> Result<ExportResult, String> {
    let handle: SharedHandle = Arc::new(JobHandle::default());
    state.jobs.lock().unwrap().insert(job.id, handle.clone());
    let jobs = state.jobs.clone();
    let fallback_dir = app.path().video_dir().ok().map(|d| d.join("Discord Ready Clip"));

    let result = tauri::async_runtime::spawn_blocking(move || {
        let id = job.id;
        let r = ffmpeg::export(&job, &handle, fallback_dir, |p| {
            let _ = app.emit("export-progress", p);
        });
        jobs.lock().unwrap().remove(&id);
        r
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}

#[tauri::command]
fn cancel_export(state: State<'_, AppState>, id: u32) {
    if let Some(h) = state.jobs.lock().unwrap().get(&id) {
        h.cancel();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let server = server::VideoServer::start().expect("couldn't start the local preview server");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState { server, jobs: Arc::default() })
        .invoke_handler(tauri::generate_handler![open_video, encoder_name, export, cancel_export])
        .run(tauri::generate_context!())
        .expect("error while running the app");
}
