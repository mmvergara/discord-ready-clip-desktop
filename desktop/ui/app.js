// Desktop UI. Trimming and cropping happen here; probing and encoding run in native ffmpeg on the Rust side.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open: openDialog } = window.__TAURI__.dialog;
const { revealItemInDir } = window.__TAURI__.opener;
const { getCurrentWebview } = window.__TAURI__.webview;

const MIN_CLIP = 0.5;
const LIMIT_KEY = 'limitMB';
const VIDEO_EXT = ['mp4', 'mkv', 'mov', 'webm', 'm4v', 'avi', 'flv', 'ts'];

const $ = (id) => document.getElementById(id);
const drop = $('drop'), dropError = $('dropError'), encEl = $('enc');
const editor = $('editor'), stage = $('stage'), video = $('video');
const cropEl = $('crop'), cropSize = $('cropSize'), cropBtn = $('cropBtn');
const playBtn = $('play'), track = $('track'), range = $('range');
const hStart = $('hStart'), hEnd = $('hEnd'), playhead = $('playhead'), clock = $('clock');
const startIn = $('start'), endIn = $('end'), len = $('len');
const exportBtn = $('export'), exportMuteBtn = $('exportMute'), queueEl = $('queue'), another = $('another');
const limitSel = $('limit');

let file = null; // { path, name, url, size, duration, fps, hasAudio } from open_video
let duration = 0, selStart = 0, selEnd = 0;
let cropOn = false;
let crop = { x: 0.1, y: 0.1, w: 0.8, h: 0.8 }; // normalized to the source frame

/* ---------------- helpers ---------------- */

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
const round1 = (t) => Math.round(t * 10) / 10;
const even = (n) => Math.max(2, Math.floor(n / 2) * 2);

function fmt(t) {
  t = round1(t);
  const m = Math.floor(t / 60);
  const s = round1(t - m * 60);
  const ss = Number.isInteger(s) ? String(s).padStart(2, '0') : s.toFixed(1).padStart(4, '0');
  return `${String(m).padStart(2, '0')}:${ss}`;
}

function fmtLen(d) {
  d = round1(d);
  return d < 60 ? `${d} sec` : fmt(d);
}

function parseTime(str) {
  const parts = str.trim().split(':');
  if (!parts.length || parts.length > 3 || parts.some((p) => p === '' || isNaN(p))) return null;
  return parts.reduce((acc, p) => acc * 60 + parseFloat(p), 0);
}

/* ---------------- settings ---------------- */

limitSel.value = localStorage.getItem(LIMIT_KEY) || '20';
if (!limitSel.value) limitSel.value = '20';
limitSel.onchange = () => localStorage.setItem(LIMIT_KEY, limitSel.value);

invoke('encoder_name')
  .then((name) => { encEl.textContent = `Encoder: ${name}`; })
  .catch((err) => { encEl.textContent = String(err); });

/* ---------------- loading a video ---------------- */

async function chooseFile() {
  const path = await openDialog({ multiple: false, directory: false, filters: [{ name: 'Video', extensions: VIDEO_EXT }] });
  if (path) openFile(path);
}
$('choose').onclick = chooseFile;
another.onclick = chooseFile;

getCurrentWebview().onDragDropEvent(({ payload }) => {
  if (payload.type === 'enter' || payload.type === 'over') drop.classList.add('over');
  else if (payload.type === 'leave') drop.classList.remove('over');
  else if (payload.type === 'drop') {
    drop.classList.remove('over');
    if (payload.paths?.[0]) openFile(payload.paths[0]);
  }
});

function showDropError(msg) {
  video.pause();
  editor.hidden = true;
  another.hidden = true;
  drop.hidden = false;
  dropError.textContent = msg;
  dropError.hidden = false;
}

async function openFile(path) {
  dropError.hidden = true;
  try {
    file = await invoke('open_video', { path });
  } catch (err) {
    file = null;
    return showDropError(String(err));
  }
  video.src = file.url;
}

video.addEventListener('loadedmetadata', () => {
  duration = video.duration;
  stage.style.setProperty('--ar', video.videoWidth / video.videoHeight);
  // Instant replay saves the moment at the end, so default to the last 30 seconds.
  selEnd = round1(duration);
  selStart = round1(Math.max(0, duration - 30));
  video.currentTime = selStart;
  drop.hidden = true;
  editor.hidden = false;
  another.hidden = false;
  render();
  renderCrop();
});

video.addEventListener('error', () => {
  if (!file) return;
  showDropError(`Couldn't preview "${file.name}". Try an MP4 (H.264) recording.`);
});

/* ---------------- playback + timeline ---------------- */

function togglePlay() {
  if (video.paused) {
    if (video.currentTime < selStart || video.currentTime >= selEnd - 0.05) video.currentTime = selStart;
    video.play();
  } else {
    video.pause();
  }
}

playBtn.onclick = togglePlay;
video.addEventListener('click', togglePlay);
video.addEventListener('play', () => playBtn.classList.add('playing'));
video.addEventListener('pause', () => playBtn.classList.remove('playing'));

const pct = (t) => `${(duration ? t / duration : 0) * 100}%`;

(function tick() {
  if (!video.paused && video.currentTime >= selEnd) video.currentTime = selStart; // loop the selection
  if (duration) {
    playhead.style.left = pct(video.currentTime);
    clock.textContent = `${fmt(video.currentTime)} / ${fmt(duration)}`;
  }
  requestAnimationFrame(tick);
})();

function render() {
  range.style.left = hStart.style.left = pct(selStart);
  hEnd.style.left = pct(selEnd);
  range.style.width = pct(selEnd - selStart);
  startIn.value = fmt(selStart);
  endIn.value = fmt(selEnd);
  len.textContent = fmtLen(selEnd - selStart);
}

function setStart(t) {
  t = clamp(round1(t), 0, duration - MIN_CLIP);
  if (t > selEnd - MIN_CLIP) selEnd = Math.min(round1(duration), t + (selEnd - selStart)); // keep the length
  selStart = t;
}

function setEnd(t) {
  selEnd = clamp(round1(t), selStart + MIN_CLIP, duration);
}

track.parentElement.addEventListener('pointerdown', (e) => {
  const mode = e.target === hStart ? 'start' : e.target === hEnd ? 'end' : 'seek';
  const el = e.currentTarget;
  el.setPointerCapture(e.pointerId);
  const move = (ev) => {
    const r = track.getBoundingClientRect();
    const t = clamp((ev.clientX - r.left) / r.width, 0, 1) * duration;
    if (mode === 'start') { setStart(Math.min(t, selEnd - MIN_CLIP)); video.currentTime = selStart; }
    else if (mode === 'end') { setEnd(t); video.currentTime = selEnd; }
    else video.currentTime = t;
    render();
  };
  const up = () => {
    el.removeEventListener('pointermove', move);
    el.removeEventListener('pointerup', up);
    el.removeEventListener('pointercancel', up);
    if (mode === 'end') video.currentTime = selStart;
  };
  if (mode === 'seek') move(e);
  el.addEventListener('pointermove', move);
  el.addEventListener('pointerup', up);
  el.addEventListener('pointercancel', up);
});

for (const [input, set] of [[startIn, setStart], [endIn, setEnd]]) {
  input.addEventListener('change', () => {
    const t = parseTime(input.value);
    if (t != null) {
      set(t);
      video.currentTime = input === startIn ? selStart : selEnd;
    }
    render();
  });
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') input.blur(); });
}

document.addEventListener('keydown', (e) => {
  if (editor.hidden || e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT' || e.metaKey || e.ctrlKey || e.altKey) return;
  const k = e.key.toLowerCase();
  if (k === ' ') { e.preventDefault(); togglePlay(); }
  else if (k === 'i') { setStart(video.currentTime); render(); }
  else if (k === 'o') { setEnd(video.currentTime); render(); }
});

/* ---------------- crop ---------------- */

const MIN_CROP = 0.05;

cropBtn.onclick = () => {
  cropOn = !cropOn;
  cropBtn.classList.toggle('active', cropOn);
  cropBtn.querySelector('span').textContent = cropOn ? 'Remove crop' : 'Crop';
  renderCrop();
};

function cropPixels() {
  const W = video.videoWidth, H = video.videoHeight;
  if (!cropOn) return { x: 0, y: 0, w: even(W), h: even(H) };
  const w = even(crop.w * W), h = even(crop.h * H);
  return {
    x: clamp(Math.round(crop.x * W), 0, W - w),
    y: clamp(Math.round(crop.y * H), 0, H - h),
    w, h,
  };
}

function renderCrop() {
  cropEl.hidden = !cropOn;
  Object.assign(cropEl.style, {
    left: `${crop.x * 100}%`, top: `${crop.y * 100}%`,
    width: `${crop.w * 100}%`, height: `${crop.h * 100}%`,
  });
  const px = cropPixels();
  cropSize.textContent = `${px.w}×${px.h}`;
}

cropEl.addEventListener('pointerdown', (e) => {
  e.preventDefault();
  const mode = e.target.dataset.h || 'move';
  const r = stage.getBoundingClientRect();
  const sx = e.clientX, sy = e.clientY, c0 = { ...crop };
  cropEl.setPointerCapture(e.pointerId);

  const move = (ev) => {
    const dx = (ev.clientX - sx) / r.width, dy = (ev.clientY - sy) / r.height;
    let { x, y, w, h } = c0;
    if (mode === 'move') {
      x = clamp(x + dx, 0, 1 - w);
      y = clamp(y + dy, 0, 1 - h);
    } else {
      if (mode.includes('w')) { const nx = clamp(x + dx, 0, x + w - MIN_CROP); w += x - nx; x = nx; }
      if (mode.includes('e')) w = clamp(w + dx, MIN_CROP, 1 - x);
      if (mode.includes('n')) { const ny = clamp(y + dy, 0, y + h - MIN_CROP); h += y - ny; y = ny; }
      if (mode.includes('s')) h = clamp(h + dy, MIN_CROP, 1 - y);
    }
    crop = { x, y, w, h };
    renderCrop();
  };
  const up = () => {
    cropEl.removeEventListener('pointermove', move);
    cropEl.removeEventListener('pointerup', up);
    cropEl.removeEventListener('pointercancel', up);
  };
  cropEl.addEventListener('pointermove', move);
  cropEl.addEventListener('pointerup', up);
  cropEl.addEventListener('pointercancel', up);
});

/* ---------------- export queue ---------------- */
// Clips encode one at a time (the GPU encoder is the bottleneck), but you can keep editing and queue more.

const queue = [];
let running = null;
let nextId = 1;

exportBtn.onclick = () => queueClip(false);
exportMuteBtn.onclick = () => queueClip(true);

function queueClip(mute) {
  if (!file) return;
  video.pause();
  const item = {
    id: nextId++, file, W: video.videoWidth, H: video.videoHeight,
    start: selStart, dur: round1(selEnd - selStart), src: cropPixels(), mute, limitMB: Number(limitSel.value),
    state: 'waiting', text: 'Waiting…', frac: null, path: null, info: '', label: null, t0: 0,
  };
  queue.push(item);
  // Clip captured; go back to the start screen so the next video can be picked while this one encodes.
  editor.hidden = true;
  another.hidden = true;
  drop.hidden = false;
  item.el = makeCard(item);
  queueEl.prepend(item.el);
  renderItem(item);
  updateExportLabel();
  pump();
}

function updateExportLabel() {
  exportBtn.textContent = running ? 'Add to Queue' : 'Export Clip';
  exportMuteBtn.textContent = running ? 'Add Without Audio' : 'Export Without Audio';
}

function makeCard(item) {
  const el = document.createElement('div');
  el.className = 'panel job';
  el.innerHTML = `
    <div class="job-head">
      <div class="job-title"></div>
      <div class="job-actions">
        <button class="btn primary job-dl" hidden>Show in Folder</button>
        <button class="link job-x"></button>
      </div>
    </div>
    <div class="job-info"></div>
    <div class="bar"><div class="bar-fill"></div></div>
    <div class="job-text"></div>`;
  el.querySelector('.job-title').textContent =
    `${item.file.name.replace(/\.[^.]+$/, '')} · ${fmt(item.start)} → ${fmt(item.start + item.dur)}${item.mute ? ' · no audio' : ''}`;
  el.querySelector('.job-x').onclick = () => removeItem(item);
  el.querySelector('.job-dl').onclick = () => revealItemInDir(item.path).catch((err) => console.error(err));
  return el;
}

function renderItem(item) {
  const el = item.el;
  el.dataset.state = item.state;
  const active = item.state === 'waiting' || item.state === 'encoding';
  el.querySelector('.bar').hidden = item.state !== 'encoding';
  el.querySelector('.bar').classList.toggle('indeterminate', active && item.frac == null);
  el.querySelector('.bar-fill').style.width = item.frac == null ? '' : `${item.frac * 100}%`;
  el.querySelector('.job-text').textContent = item.state === 'done' ? item.path : item.text;
  el.querySelector('.job-info').textContent = item.info;
  el.querySelector('.job-x').textContent = active ? 'Cancel' : 'Remove';
  el.querySelector('.job-dl').hidden = !item.path;
}

function setItem(item, text, frac = null) {
  item.text = text;
  item.frac = frac;
  renderItem(item);
}

function removeItem(item) {
  queue.splice(queue.indexOf(item), 1);
  item.el.remove();
  if (item === running) {
    item.state = 'cancelled';
    invoke('cancel_export', { id: item.id }); // the in-flight export rejects; pump() moves on
  }
}

function etaText(t0, frac) {
  const elapsed = (performance.now() - t0) / 1000;
  const left = frac > 0.03 ? Math.round(elapsed / frac - elapsed) : null;
  return left == null ? '' : left >= 60 ? ` · about ${Math.ceil(left / 60)} min left` : ` · ${left}s left`;
}

listen('export-progress', ({ payload }) => {
  const item = queue.find((q) => q.id === payload.id);
  if (!item || item.state !== 'encoding') return;
  if (payload.frac < 0) return setItem(item, `${payload.label}…`);
  if (item.label !== payload.label) { item.label = payload.label; item.t0 = performance.now(); }
  setItem(item, `${payload.label}… ${Math.round(payload.frac * 100)}%${etaText(item.t0, payload.frac)}`, payload.frac);
});

async function pump() {
  if (running) return;
  const item = queue.find((q) => q.state === 'waiting');
  if (!item) return updateExportLabel();
  running = item;
  item.state = 'encoding';
  updateExportLabel();
  setItem(item, 'Starting…');
  try {
    const r = await invoke('export', {
      job: {
        id: item.id, path: item.file.path, start: item.start, dur: item.dur, src: item.src,
        width: item.W, height: item.H, mute: item.mute, limitBytes: item.limitMB * 1e6,
      },
    });
    item.state = 'done';
    item.path = r.path;
    item.info = [
      `${(r.size / 1e6).toFixed(1)} MB`, fmtLen(item.dur), `${r.width}×${r.height}`,
      r.passthrough ? 'original, no re-encode' : r.encoder,
    ].join(' · ');
    renderItem(item);
  } catch (err) {
    if (item.state !== 'cancelled') {
      console.error(err);
      item.state = 'failed';
      setItem(item, `Export failed: ${err}`);
    }
  }
  running = null;
  pump();
}
