<p align="center">
  <img src="src-tauri/icons/128x128.png" alt="Discord Ready Clip icon" width="96" height="96">
</p>

<h1 align="center">Discord Ready Clip</h1>

<p align="center">
  <strong>Compress, trim and crop a video clip to fit Discord's upload limit (10 MB, 20 MB or 50 MB) in a couple of clicks.</strong><br>
  A free desktop app for Windows and Linux with GPU encoding and no watermark.
</p>

<p align="center">
  <a href="../../releases/latest"><strong>⬇ Download for Windows / Linux</strong></a>
</p>



https://github.com/user-attachments/assets/6a124c8e-738a-4bdb-abd5-ab57bd0aa448




---

Discord rejects files over its size limit, so a gameplay clip usually needs to be cut down and re-encoded before you can share it. Discord Ready Clip does both steps. Open a recording from NVIDIA ShadowPlay, OBS replay buffer, AMD ReLive, Xbox Game Bar or anything else, pick the moment, and export an MP4 that is just under the limit you choose. Because it targets the limit, you get the best quality that will still upload.

## Features

- **Fits Discord's file size limit.** Pick 10 MB (free), 20 MB or 50 MB (Nitro tiers). The app calculates the bitrate for that size and re-encodes once if the first pass misses.
- **Fast GPU encoding.** It uses NVIDIA NVENC, Intel Quick Sync or AMD AMF through native ffmpeg, with CPU x264 as a fallback. A 30-second 1080p60 clip takes a few seconds.
- **Trim on a timeline.** The selection starts on the last 30 seconds, which is where instant replay puts the moment. Drag the handles or type exact times.
- **Crop** to a region of the frame, for example to cut out the HUD or focus on one monitor.
- **Export with or without audio.**
- **Queue several clips** from the same recording while one is encoding.
- **Automatic downscaling.** If the bitrate is too low for the resolution, it scales the video down just enough to keep it sharp, and never below 360p.
- **Works offline.** Nothing is uploaded and there's no account or watermark.
- **Opens common formats:** MP4, MKV, MOV, WebM, M4V, AVI, FLV and TS.

## Download and install

Get the latest installer from the [**Releases page**](../../releases/latest).

- **Windows 10/11:** run the `-setup.exe`. ffmpeg is included. The installer isn't code-signed, so if SmartScreen warns you, choose "More info" → "Run anyway".
- **Linux (Debian, Ubuntu, Mint, Pop!_OS):** `sudo apt install ./Discord.Ready.Clip_<version>_amd64.deb`. This also installs ffmpeg.

## How to make a clip small enough for Discord

1. Drop a video on the window, or click **Choose Video**.
2. Drag the timeline handles, or type start and end times. **Space** plays and pauses; **I** and **O** set the start and end at the playhead.
3. Optionally click **Crop** and drag the box.
4. Pick the size limit (**Max**) and click **Export Clip**, or **Export Without Audio**.
5. Drag the finished MP4 into Discord.

Clips are saved next to the original as `<name>-clip-<mmss>.mp4`. **Show in Folder** opens that folder. If the original folder isn't writable, clips go to `Videos/Discord Ready Clip`.

## FAQ

### What is Discord's file size limit?

Free accounts can upload files up to 10 MB. Nitro Basic and server boosts raise that, and Nitro raises it further. Discord Ready Clip offers 10, 20 and 50 MB targets; pick the one that matches what you can upload.

### How do I compress a video for Discord without losing quality?

Keep the clip short and let the app target the limit. A shorter clip gets more bitrate per second, so trimming to the moment you want is the biggest quality win. The app spends nearly all of the allowed size on the video, and only lowers the resolution when the bitrate would be too thin for it.

### Does it work with NVIDIA ShadowPlay / Instant Replay?

Yes. ShadowPlay saves the last few minutes, and the app selects the final 30 seconds by default, so most of the time you just adjust the handles and export. OBS replay buffer, AMD ReLive and Xbox Game Bar recordings work the same way.

### Do I need a GPU?

No. If no hardware encoder works, it falls back to CPU x264, which is slower but produces the same file size.

### Is there a macOS version?

Not yet. Builds are published for Windows and Linux.

## How the compression works

- On first use, the app test-encodes with each H.264 encoder and keeps the first that works: NVIDIA NVENC, Intel Quick Sync, AMD AMF, then CPU x264. The header shows which one it picked.
- Video bitrate = (97% of the limit × 8 ÷ clip length) − 128 kbps audio, minus about 1.5% for MP4 overhead.
- If that bitrate is too thin for the resolution (under 0.035 bits per pixel per frame), the output is scaled down just enough, never below 360p.
- If the result is over the limit, or uses less than 85% of the target, it re-encodes once at a corrected bitrate.
- If the clip is already under the limit with nothing trimmed or cropped, the original file is used as-is.

## Development

Built with [Tauri 2](https://tauri.app/) (Rust) and a plain HTML/CSS/JS front end. Requirements: Node 20+, Rust (stable), and ffmpeg on PATH. On Linux, also the [Tauri system packages](https://v2.tauri.app/start/prerequisites/#linux).

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

### Releases

`.github/workflows/release.yml` builds the Windows installer (downloading ffmpeg to bundle) and the Linux `.deb`. It publishes them to the GitHub release named after `version` in `src-tauri/tauri.conf.json`. This runs on every push to `main` that changes anything other than Markdown files, and can also be started from the Actions tab. Pushing again with the same version replaces that release's files; bump the version to publish a new release.

---

*Discord Ready Clip is an independent project and is not affiliated with or endorsed by Discord Inc.*
