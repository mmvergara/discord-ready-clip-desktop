# Discord Ready Clip

A desktop app for Windows and Linux. Open a ShadowPlay (or any) recording, trim it, optionally crop it, and export a clip that fits Discord's upload limit: 10, 20 or 50 MB. Encoding runs on your GPU through native ffmpeg, so a 30-second 1080p60 clip takes seconds.

## Install

Download the latest installer from [Releases](../../releases/latest).

- **Windows:** run the `-setup.exe`. ffmpeg is included. The installer isn't code-signed, so if SmartScreen warns, choose "More info" → "Run anyway".
- **Linux (Debian, Ubuntu, Mint):** `sudo apt install ./Discord.Ready.Clip_<version>_amd64.deb`. This also installs ffmpeg.

## Using it

1. Drop a video on the window, or click **Choose Video**. The selection starts as the last 30 seconds, where instant replay puts the moment.
2. Drag the timeline handles, or type start and end times. **Space** plays and pauses; **I** and **O** set the start and end at the playhead.
3. Optionally click **Crop** and drag the box.
4. Pick the size limit (**Max**) and click **Export Clip**, or **Export Without Audio**. You can queue more clips while one encodes.

Clips are saved next to the original as `<name>-clip-<mmss>.mp4`. **Show in Folder** opens that folder. If the original folder isn't writable, clips go to `Videos/Discord Ready Clip`.

## How compression works

- On first use, the app test-encodes with each H.264 encoder and keeps the first that works: NVIDIA NVENC, Intel Quick Sync, AMD AMF, then CPU x264. The header shows which one it picked.
- Video bitrate = (97% of the limit × 8 ÷ clip length) − 128 kbps audio, minus ~1.5% for MP4 overhead.
- If that bitrate is too thin for the resolution (under 0.035 bits per pixel per frame), the output is scaled down just enough, never below 360p.
- If the result is over the limit, or uses less than 85% of the target, it re-encodes once at a corrected bitrate.
- If the clip is already under the limit with nothing trimmed or cropped, the original file is used as-is.

## Development

Requirements: Node 20+, Rust (stable), and ffmpeg on PATH. On Linux, also the [Tauri system packages](https://v2.tauri.app/start/prerequisites/#linux).

```sh
npm ci
npx tauri dev            # run the app
npx tauri build          # build the installer for this OS
cd src-tauri && cargo test -- --include-ignored   # tests (the ignored one runs a real ffmpeg export)
```

- `ui/`: the editor (plain HTML, CSS and JS, no bundler)
- `src-tauri/src/ffmpeg.rs`: probing, encoder detection, size-targeted encoding
- `src-tauri/src/server.rs`: loopback HTTP server that streams the opened video to the preview player
- `src-tauri/src/lib.rs`: commands the UI calls

## Releases

`.github/workflows/release.yml` builds the Windows installer (downloading ffmpeg to bundle) and the Linux `.deb`. It publishes them to the GitHub release named after `version` in `src-tauri/tauri.conf.json`. This runs on every push to `main` that changes anything other than Markdown files, and can also be started from the Actions tab. Pushing again with the same version replaces that release's files; bump the version to publish a new release.
