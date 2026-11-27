//! Native ffmpeg: locating the binary, probing inputs, picking a hardware encoder, and encoding a clip
//! so it lands just under a byte limit.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const AUDIO_BPS: f64 = 128e3;
/// Below this many bits per pixel per frame, downscale instead of starving quality (keeps 1080p60 at ~5 Mbps).
const MIN_BPP: f64 = 0.035;
const MIN_HEIGHT: f64 = 360.0;
/// Aim this far under the limit so muxing overhead and rate-control drift don't push the file over.
const TARGET_RATIO: f64 = 0.97;
/// A first encode smaller than this share of the target wasted space worth a second, higher-bitrate pass.
const UNDERSHOOT: f64 = 0.85;

/* ---------------- running ffmpeg ---------------- */

/// The bundled binary next to the app executable (Windows installer), else `ffmpeg` from PATH (Linux package).
fn ffmpeg_path() -> PathBuf {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        let bundled = dir.join(name);
        if bundled.is_file() {
            return bundled;
        }
    }
    PathBuf::from(name)
}

fn ffmpeg() -> Command {
    let mut cmd = Command::new(ffmpeg_path());
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn missing_ffmpeg(err: std::io::Error) -> String {
    if cfg!(windows) {
        format!("Couldn't start ffmpeg ({err}). Try reinstalling the app.")
    } else {
        format!("Couldn't start ffmpeg ({err}). Install it with: sudo apt install ffmpeg")
    }
}

/* ---------------- probing ---------------- */

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
    pub size: u64,
}

pub fn probe(path: &Path) -> Result<Probe, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't read the file: {e}"))?.len();
    // `ffmpeg -i` with no output "fails", but prints the stream info we need on stderr.
    let out = ffmpeg()
        .arg("-hide_banner")
        .arg("-i")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(missing_ffmpeg)?;
    let text = String::from_utf8_lossy(&out.stderr);

    static DURATION: OnceLock<Regex> = OnceLock::new();
    static DIMS: OnceLock<Regex> = OnceLock::new();
    static FPS: OnceLock<Regex> = OnceLock::new();
    let duration_re = DURATION.get_or_init(|| Regex::new(r"Duration: (\d+):(\d+):(\d+(?:\.\d+)?)").unwrap());
    let dims_re = DIMS.get_or_init(|| Regex::new(r"\b(\d{2,5})x(\d{2,5})\b").unwrap());
    let fps_re = FPS.get_or_init(|| Regex::new(r"([\d.]+) (?:fps|tbr)").unwrap());

    let video_line = text
        .lines()
        .find(|l| l.contains("Stream #") && l.contains("Video:"))
        .ok_or("No video stream found in this file.")?;
    let duration = duration_re
        .captures(&text)
        .map(|c| c[1].parse::<f64>().unwrap() * 3600.0 + c[2].parse::<f64>().unwrap() * 60.0 + c[3].parse::<f64>().unwrap())
        .unwrap_or(0.0);
    let (width, height) = dims_re
        .captures(video_line)
        .map(|c| (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0)))
        .unwrap_or((0, 0));
    let fps = fps_re
        .captures(video_line)
        .and_then(|c| c[1].parse::<f64>().ok())
        .filter(|f| *f > 0.0)
        .unwrap_or(60.0);
    let has_audio = text.lines().any(|l| l.contains("Stream #") && l.contains("Audio:"));

    Ok(Probe { duration, width, height, fps, has_audio, size })
}

/* ---------------- encoder selection ---------------- */

pub struct Encoder {
    pub label: &'static str,
    args: &'static [&'static str],
}

/// Tried in order; the first one that can encode on this machine is used for every export.
/// NVENC is listed twice: older drivers reject multipass/B-frames but still do plain VBR.
const ENCODERS: &[Encoder] = &[
    Encoder {
        label: "NVIDIA NVENC",
        args: &["-c:v", "h264_nvenc", "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-multipass", "fullres",
                "-spatial-aq", "1", "-bf", "3", "-profile:v", "high", "-pix_fmt", "yuv420p"],
    },
    Encoder {
        label: "NVIDIA NVENC",
        args: &["-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-profile:v", "high", "-pix_fmt", "yuv420p"],
    },
    Encoder {
        label: "Intel Quick Sync",
        args: &["-c:v", "h264_qsv", "-preset", "slower", "-profile:v", "high", "-pix_fmt", "nv12"],
    },
    Encoder {
        label: "AMD AMF",
        args: &["-c:v", "h264_amf", "-usage", "transcoding", "-quality", "quality", "-rc", "vbr_peak",
                "-profile:v", "high", "-pix_fmt", "nv12"],
    },
    Encoder {
        label: "CPU (x264)",
        args: &["-c:v", "libx264", "-preset", "medium", "-profile:v", "high", "-pix_fmt", "yuv420p"],
    },
];

fn rate_args(vbps: f64) -> Vec<String> {
    let k = (vbps / 1000.0).round() as u64;
    vec![
        "-b:v".into(), format!("{k}k"),
        "-maxrate".into(), format!("{}k", k * 3 / 2),
        "-bufsize".into(), format!("{}k", k * 2),
    ]
}

fn encoder_works(enc: &Encoder) -> bool {
    let mut cmd = ffmpeg();
    cmd.args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=1280x720:r=30",
              "-frames:v", "10"])
        .args(enc.args)
        .args(rate_args(2e6))
        .args(["-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = cmd.spawn() else { return false };
    // A broken driver can hang the encoder instead of failing; don't let detection block forever.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub fn encoder() -> &'static Encoder {
    static CHOSEN: OnceLock<&'static Encoder> = OnceLock::new();
    CHOSEN.get_or_init(|| ENCODERS.iter().find(|e| encoder_works(e)).unwrap_or(ENCODERS.last().unwrap()))
}

/* ---------------- export ---------------- */

#[derive(Clone, Copy, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: u32,
    pub path: String,
    pub start: f64,
    pub dur: f64,
    /// Area of the source frame to keep, in pixels.
    pub src: Rect,
    /// Displayed size of the source frame, so an untouched crop can be recognized.
    pub width: u32,
    pub height: u32,
    pub mute: bool,
    pub limit_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub encoder: String,
    pub passthrough: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: u32,
    pub label: String,
    pub frac: f64,
}

/// Lets a running export be cancelled from another command.
#[derive(Default)]
pub struct JobHandle {
    child: Mutex<Option<Child>>,
    cancelled: AtomicBool,
}

impl JobHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

pub const CANCELLED: &str = "cancelled";

fn even(n: f64) -> u32 {
    let n = n as u32;
    if n < 2 { 2 } else { n / 2 * 2 }
}

/// Bitrate that lands the whole file near `target` bytes, leaving a little room for MP4 overhead.
fn video_bitrate_for(dur: f64, has_audio: bool, target: f64) -> f64 {
    let total = target * 8.0 * 0.985 / dur;
    (total - if has_audio { AUDIO_BPS } else { 0.0 }).max(300e3)
}

/// Keep the crop's resolution unless the bitrate is too thin for it, then scale down just enough.
fn output_size(src: Rect, vbps: f64, fps: f64) -> (u32, u32) {
    let (w, h) = (src.w as f64, src.h as f64);
    let bpp = vbps / (w * h * fps.min(60.0));
    let mut s = (bpp / MIN_BPP).sqrt().min(1.0);
    s = s.max((MIN_HEIGHT / w.min(h)).min(1.0));
    (even(w * s), even(h * s))
}

/// `<source folder>/<name>-clip-<mmss>.mp4`, numbered if that name is taken. Falls back to `fallback_dir`
/// when the source folder can't be written to.
fn output_path(job: &Job, fallback_dir: Option<PathBuf>) -> PathBuf {
    let src = Path::new(&job.path);
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into());
    let t = (job.start * 10.0).round() / 10.0;
    let stamp = format!("{:02}{:02}", (t / 60.0) as u32, (t % 60.0) as u32);

    let mut dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    let probe = dir.join(".discord-ready-clip-write-test");
    if std::fs::File::create(&probe).is_ok() {
        let _ = std::fs::remove_file(&probe);
    } else if let Some(fb) = fallback_dir {
        let _ = std::fs::create_dir_all(&fb);
        dir = fb;
    }

    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}-clip-{stamp}.mp4") } else { format!("{stem}-clip-{stamp} ({n}).mp4") };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

struct Attempt {
    path: PathBuf,
    size: u64,
    out: (u32, u32),
}

pub fn export(
    job: &Job,
    handle: &JobHandle,
    fallback_dir: Option<PathBuf>,
    on_progress: impl Fn(Progress),
) -> Result<ExportResult, String> {
    let report = |label: &str, frac: f64| on_progress(Progress { id: job.id, label: label.into(), frac });
    report("Reading video", -1.0);
    let info = probe(Path::new(&job.path))?;
    let limit = job.limit_bytes as f64;
    let target = limit * TARGET_RATIO;
    let crop_on = job.src.w != even(job.width as f64) || job.src.h != even(job.height as f64);
    let untouched = !job.mute && !crop_on && job.start <= 0.05 && job.dur >= info.duration - 0.15;

    // Already small enough with nothing to trim or crop: the original is the answer.
    if untouched && info.size as f64 <= target {
        return Ok(ExportResult {
            path: job.path.clone(),
            size: info.size,
            width: job.width,
            height: job.height,
            encoder: String::new(),
            passthrough: true,
        });
    }

    let has_audio = info.has_audio && !job.mute;
    let enc = encoder();
    let final_path = output_path(job, fallback_dir);
    let part = |n: u32| {
        let mut name = final_path.file_name().unwrap().to_os_string();
        name.push(format!(".part{n}"));
        final_path.with_file_name(name)
    };
    let cleanup = |paths: &[&PathBuf]| paths.iter().for_each(|p| { let _ = std::fs::remove_file(p); });

    let attempt = |n: u32, vbps: f64, label: &str| -> Result<Attempt, String> {
        let out = output_size(job.src, vbps, info.fps);
        let path = part(n);
        let result = encode(job, handle, enc, &path, out, vbps, has_audio, |frac| report(label, frac));
        match result {
            Ok(()) => Ok(Attempt { size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0), path, out }),
            Err(e) => {
                cleanup(&[&path]);
                Err(e)
            }
        }
    };

    // Encode at the bitrate that should land near the target, then correct once by how far off it landed:
    // down if it went over the limit, up if it wasted a lot of the space (hardware encoders often undershoot).
    let mut vbps = video_bitrate_for(job.dur, has_audio, target);
    let first = attempt(1, vbps, "Encoding")?;
    let over = first.size as f64 > limit;
    let best = if !over && first.size as f64 >= target * UNDERSHOOT {
        first
    } else {
        vbps *= target / first.size.max(1) as f64 * if over { 0.92 } else { 0.97 };
        let label = if over { "Too big, re-encoding at a lower bitrate" } else { "Using the leftover space for better quality, re-encoding" };
        let second = match attempt(2, vbps, label) {
            Ok(a) => a,
            Err(e) => {
                cleanup(&[&first.path]);
                return Err(e);
            }
        };
        if second.size as f64 <= limit {
            cleanup(&[&first.path]);
            second
        } else if !over {
            cleanup(&[&second.path]);
            first
        } else {
            cleanup(&[&first.path, &second.path]);
            return Err(format!(
                "Couldn't get the clip under {} MB. Try a shorter selection.",
                (limit / 1e6).round()
            ));
        }
    };

    std::fs::rename(&best.path, &final_path).map_err(|e| {
        cleanup(&[&best.path]);
        format!("Couldn't save the clip: {e}")
    })?;
    Ok(ExportResult {
        path: final_path.to_string_lossy().into_owned(),
        size: best.size,
        width: best.out.0,
        height: best.out.1,
        encoder: enc.label.into(),
        passthrough: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn encode(
    job: &Job,
    handle: &JobHandle,
    enc: &Encoder,
    out_path: &Path,
    out: (u32, u32),
    vbps: f64,
    has_audio: bool,
    on_frac: impl Fn(f64),
) -> Result<(), String> {
    if handle.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let src = job.src;
    let mut filters = Vec::new();
    if src.w != job.width || src.h != job.height {
        filters.push(format!("crop={}:{}:{}:{}", src.w, src.h, src.x, src.y));
    }
    if out != (src.w, src.h) {
        filters.push(format!("scale={}:{}", out.0, out.1));
    }
    filters.push("setsar=1".into());

    let mut cmd = ffmpeg();
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-progress", "pipe:1", "-nostats"])
        .args(["-ss", &format!("{:.3}", job.start)])
        .arg("-i")
        .arg(&job.path)
        .args(["-t", &format!("{:.3}", job.dur), "-map", "0:v:0"]);
    if has_audio {
        cmd.args(["-map", "0:a:0?"]);
    }
    cmd.args(["-vf", &filters.join(",")]).args(enc.args).args(rate_args(vbps));
    if has_audio {
        cmd.args(["-c:a", "aac", "-b:a", &format!("{}k", (AUDIO_BPS / 1000.0) as u32), "-ac", "2"]);
    }
    cmd.args(["-movflags", "+faststart", "-f", "mp4", "-y"])
        .arg(out_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(missing_ffmpeg)?;
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    *handle.child.lock().unwrap() = Some(child);
    if handle.is_cancelled() {
        handle.cancel(); // cancelled between the check above and the spawn
    }

    let errors = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    on_frac(0.0);
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        // `out_time_us` (and, despite its name, `out_time_ms`) is the output position in microseconds.
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
            on_frac((us / 1e6 / job.dur).clamp(0.0, 1.0));
        }
    }

    let status = handle.child.lock().unwrap().take().unwrap().wait();
    let errors = errors.join().unwrap_or_default();
    if handle.is_cancelled() {
        return Err(CANCELLED.into());
    }
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => {
            let detail = errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
            Err(if detail.is_empty() { format!("ffmpeg exited with {s}") } else { detail })
        }
        Err(e) => Err(format!("ffmpeg failed: {e}")),
    }
}

/// Shared handle type stored per running export.
pub type SharedHandle = Arc<JobHandle>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_resolution_when_bitrate_allows() {
        let src = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        assert_eq!(output_size(src, 8e6, 60.0), (1920, 1080));
    }

    #[test]
    fn downscales_thin_bitrates_but_not_below_360p() {
        let src = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        let (w, h) = output_size(src, 2e6, 60.0);
        assert!(h < 1080 && h >= 360, "{w}x{h}");
        assert_eq!(output_size(src, 1e5, 60.0).1, 360);
    }

    #[test]
    fn bitrate_fills_the_target() {
        let vbps = video_bitrate_for(30.0, true, 19.4e6);
        let bytes = (vbps + AUDIO_BPS) * 30.0 / 8.0;
        assert!(bytes < 19.4e6 && bytes > 18.5e6, "{bytes}");
    }

    /// Needs ffmpeg on PATH: `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn exports_under_the_limit() {
        let dir = std::env::temp_dir().join(format!("drc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("game.mp4");
        let ok = ffmpeg()
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=s=1920x1080:r=60:d=40,noise=alls=40:allf=t",
                   "-f", "lavfi", "-i", "sine=f=440:d=40", "-c:v", "libx264", "-preset", "ultrafast", "-b:v", "30M",
                   "-c:a", "aac", "-shortest", "-y"])
            .arg(&input)
            .status()
            .unwrap()
            .success();
        assert!(ok);

        let job = Job {
            id: 1, path: input.to_string_lossy().into(), start: 10.0, dur: 25.0,
            src: Rect { x: 0, y: 0, w: 1920, h: 1080 }, width: 1920, height: 1080, mute: false,
            limit_bytes: 10_000_000,
        };
        let t = Instant::now();
        let r = export(&job, &JobHandle::default(), None, |p| eprintln!("{} {:.0}%", p.label, p.frac * 100.0)).unwrap();
        eprintln!("{} bytes, {}x{}, {}, {:.1}s", r.size, r.width, r.height, r.encoder, t.elapsed().as_secs_f64());
        assert!(r.size <= 10_000_000 && r.size as f64 > 10e6 * TARGET_RATIO * UNDERSHOOT * 0.9);
        assert!(r.path.ends_with("game-clip-0010.mp4"));
        let p = probe(Path::new(&r.path)).unwrap();
        assert!((p.duration - 25.0).abs() < 0.2 && p.has_audio, "{}", p.duration);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
