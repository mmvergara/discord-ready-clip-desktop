# Discord Ready Clip

Upload → Trim → Crop → Compress → Download. A static page that cuts a ShadowPlay recording down to a clip under 20 MB (aiming for ~18 MB), entirely in the browser with [ffmpeg.wasm](https://github.com/ffmpegwasm/ffmpeg.wasm). Videos are never uploaded anywhere.

## Files

- `index.html`, `style.css`, `app.js`: the whole app
- `vendor/ffmpeg/`: `@ffmpeg/ffmpeg@0.12.15` ESM build, served same-origin so its Web Worker can load
- The ffmpeg core (~32 MB) is fetched from jsDelivr on first use and cached by the browser. When the page is cross-origin isolated (the COOP/COEP headers in `vercel.json`), it uses the multi-threaded `@ffmpeg/core-mt` (about 2x faster); otherwise, or if that core stalls, it falls back to the single-threaded `@ffmpeg/core`

## Run locally

Any static server works, e.g. `python3 -m http.server 8000`, then open http://localhost:8000. Plain servers don't send the COOP/COEP headers, so locally it runs the slower single-threaded core; `npx vercel dev` applies them.

## Deploy to Vercel

No build step. Import the repo in Vercel with framework preset "Other" (leave the build command empty), or run `npx vercel` in this folder.

## How compression works

- Video bitrate = (18 MB × 8 ÷ clip length) − 128 kbps audio, minus ~2% for MP4 overhead
- If that bitrate is too thin for the crop's resolution (under ~0.06 bits per pixel per frame), the output is scaled down just enough, never below 360p
- H.264 (libx264) + AAC in MP4 with faststart
- If the result is over 20 MB, it re-encodes at a proportionally lower bitrate until it fits

## Desktop app (`desktop/`)

The same editor as a Tauri app that runs native ffmpeg instead of ffmpeg.wasm. On first use it test-encodes with each hardware encoder and keeps the first that works: NVIDIA NVENC, then Intel Quick Sync, then AMD AMF, then CPU x264. Clips are saved next to the source recording as `<name>-clip-<mmss>.mp4`, with a 10, 20 or 50 MB size limit.

- **Windows:** installer with ffmpeg bundled. Every push to `main` that changes `desktop/` builds it and publishes it to the GitHub release for the app version (`version` in `desktop/src-tauri/tauri.conf.json`); bump the version for a new release.
- **Linux:** a `.deb` that depends on the system `ffmpeg` package. Build with `cd desktop && npm ci && npx tauri build`, then `sudo apt install "./src-tauri/target/release/bundle/deb/Discord Ready Clip_0.1.0_amd64.deb"`.
- **Dev:** `cd desktop && npm ci && npx tauri dev` (Linux uses ffmpeg from PATH). Backend tests: `cd desktop/src-tauri && cargo test -- --include-ignored`.
