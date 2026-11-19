'use strict';

/* RustCade front-end: boots the Rust WebAssembly cores (CHIP-8 and Game Boy),
   renders their framebuffers and feeds touch / keyboard input back into them. */

const FRAME_MS = 1000 / 60;
const SPEEDS = [
  { cycles: 5, label: '0.4×' },
  { cycles: 12, label: '1×' },
  { cycles: 24, label: '2×' },
];

// Game Boy DMG shades (0 = lightest) and the CHIP-8 phosphor pair.
const PALETTES = {
  chip8: [
    [9, 20, 15],
    [150, 255, 178],
  ],
  gb: [
    [155, 188, 15],
    [139, 172, 15],
    [48, 98, 48],
    [15, 56, 15],
  ],
};

const KEYPAD_LAYOUT = [
  ['1', '2', '3', 'C'],
  ['4', '5', '6', 'D'],
  ['7', '8', '9', 'E'],
  ['A', '0', 'B', 'F'],
];

const els = {
  app: document.getElementById('app'),
  screen: document.getElementById('screen'),
  overlay: document.getElementById('overlay'),
  badge: document.getElementById('badge'),
  roms: document.getElementById('roms'),
  hint: document.getElementById('hint'),
  keypad: document.getElementById('keypad'),
  fps: document.getElementById('fps'),
  hz: document.getElementById('hz'),
  hzUnit: document.getElementById('hzUnit'),
  sysline: document.getElementById('sysline'),
  pause: document.getElementById('btnPause'),
  reset: document.getElementById('btnReset'),
  save: document.getElementById('btnSave'),
  load: document.getElementById('btnLoad'),
  pad: document.getElementById('btnPad'),
  speed: document.getElementById('btnSpeed'),
  full: document.getElementById('btnFull'),
  file: document.getElementById('file'),
  toast: document.getElementById('toast'),
};

const app = {
  wasm: null,
  memory: null,
  ready: false,
  romId: null,
  romName: 'No ROM loaded',
  system: 'chip8',
  paused: false,
  halted: false,
  speed: 1,
  games: [],
};

/* ---------------------------------------------------------------- input -- */

const pressed = new Set();
const keyDescs = {};

function toKey(value) {
  if (value === undefined || value === null || value === '') return null;
  const parsed = parseInt(value, 16);
  return Number.isNaN(parsed) ? null : parsed;
}

function mapKey(code, chip, gb) {
  keyDescs[code] = { chip, gb };
}

// CHIP-8 hex pad: 1 2 3 C / 4 5 6 D / 7 8 9 E / A 0 B F.
mapKey('Digit1', 0x1, null);
mapKey('Digit2', 0x2, null);
mapKey('Digit3', 0x3, null);
mapKey('Digit4', 0xc, null);
mapKey('KeyQ', 0x4, null);
mapKey('KeyW', 0x5, null);
mapKey('KeyE', 0x6, null);
mapKey('KeyR', 0xd, null);
mapKey('KeyA', 0x7, null);
mapKey('KeyS', 0x8, null);
mapKey('KeyD', 0x9, null);
mapKey('KeyF', 0xe, null);
mapKey('KeyC', 0xb, null);
mapKey('KeyV', 0xf, null);
// Game Boy: arrows, X = A, Z = B, Enter = start, Shift/Backspace = select.
mapKey('ArrowUp', null, 2);
mapKey('ArrowDown', null, 3);
mapKey('ArrowLeft', null, 1);
mapKey('ArrowRight', null, 0);
mapKey('KeyX', 0x0, 4);
mapKey('KeyZ', 0xa, 5);
mapKey('Enter', null, 7);
mapKey('NumpadEnter', null, 7);
mapKey('ShiftLeft', null, 6);
mapKey('ShiftRight', null, 6);
mapKey('Backspace', null, 6);

function keyMask() {
  let mask = 0;
  for (const desc of pressed) {
    const key = app.system === 'gb' ? desc.gb : desc.chip;
    if (key !== null && key !== undefined) mask |= 1 << key;
  }
  return mask >>> 0;
}

function press(desc, down) {
  if (down) pressed.add(desc);
  else pressed.delete(desc);
}

function bindKey(el, desc) {
  const pointers = new Set();
  const release = (event) => {
    pointers.delete(event.pointerId);
    if (pointers.size === 0) {
      press(desc, false);
      el.classList.remove('down');
    }
  };
  el.addEventListener('pointerdown', (event) => {
    event.preventDefault();
    unlockAudio();
    if (el.setPointerCapture) {
      try {
        el.setPointerCapture(event.pointerId);
      } catch {}
    }
    pointers.add(event.pointerId);
    press(desc, true);
    el.classList.add('down');
  });
  el.addEventListener('pointerup', release);
  el.addEventListener('pointercancel', release);
  el.addEventListener('lostpointercapture', release);
  el.addEventListener('contextmenu', (event) => event.preventDefault());
}

function wirePad() {
  document.querySelectorAll('[data-chip], [data-gb]').forEach((el) => {
    bindKey(el, {
      chip: toKey(el.dataset.chip),
      gb: toKey(el.dataset.gb),
    });
  });
  for (const row of KEYPAD_LAYOUT) {
    for (const label of row) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'keypad-key';
      button.textContent = label;
      button.setAttribute('aria-label', `Key ${label}`);
      bindKey(button, { chip: parseInt(label, 16), gb: null });
      els.keypad.appendChild(button);
    }
  }
}

function wireKeyboard() {
  window.addEventListener('keydown', (event) => {
    const desc = keyDescs[event.code];
    if (desc) {
      event.preventDefault();
      unlockAudio();
      press(desc, true);
      return;
    }
    if (event.code === 'Space') {
      event.preventDefault();
      togglePause();
    } else if (event.code === 'Escape') {
      els.keypad.hidden = true;
    }
  });
  window.addEventListener('keyup', (event) => {
    const desc = keyDescs[event.code];
    if (desc) {
      event.preventDefault();
      press(desc, false);
    }
  });
  window.addEventListener('blur', () => pressed.clear());
}

/* --------------------------------------------------------------- audio -- */

let audio = null;

function unlockAudio() {
  if (audio) {
    if (audio.ctx.state === 'suspended') audio.ctx.resume();
    return;
  }
  const AudioCtx = window.AudioContext || window.webkitAudioContext;
  if (!AudioCtx) return;
  const audioCtx = new AudioCtx();
  const osc = audioCtx.createOscillator();
  const gain = audioCtx.createGain();
  osc.type = 'square';
  osc.frequency.value = 440;
  gain.gain.value = 0;
  osc.connect(gain);
  gain.connect(audioCtx.destination);
  osc.start();
  audio = { ctx: audioCtx, osc, gain, on: false };
}

function setBuzzer(on) {
  if (!audio) return;
  if (audio.on === on) return;
  audio.on = on;
  audio.gain.gain.setTargetAtTime(on ? 0.045 : 0, audio.ctx.currentTime, 0.006);
}

/* -------------------------------------------------------------- render -- */

function render() {
  if (!app.wasm) return;
  const width = app.wasm.rc_width();
  const height = app.wasm.rc_height();
  const stride = app.wasm.rc_stride();
  if (offscreen.width !== width || offscreen.height !== height) {
    offscreen.width = width;
    offscreen.height = height;
    image = octx.createImageData(width, height);
  }
  const frame = new Uint8Array(app.memory.buffer, app.wasm.rc_fb_ptr(), stride * height);
  const palette = PALETTES[app.system] || PALETTES.chip8;
  const pixels = image.data;
  let offset = 0;
  for (let y = 0; y < height; y++) {
    const row = y * stride;
    for (let x = 0; x < width; x++) {
      const shade = palette[frame[row + x]] || palette[0];
      pixels[offset] = shade[0];
      pixels[offset + 1] = shade[1];
      pixels[offset + 2] = shade[2];
      pixels[offset + 3] = 255;
      offset += 4;
    }
  }
  octx.putImageData(image, 0, 0);

  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const cssWidth = els.screen.clientWidth || 320;
  const targetW = Math.max(64, Math.round(cssWidth * dpr));
  const targetH = Math.max(32, Math.round((targetW * height) / width));
  if (els.screen.width !== targetW || els.screen.height !== targetH) {
    els.screen.width = targetW;
    els.screen.height = targetH;
  }
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(offscreen, 0, 0, els.screen.width, els.screen.height);
  els.badge.textContent = `${app.romName} · ${app.system === 'gb' ? 'DMG' : 'CHIP-8'} ${width}×${height}`;
}

const offscreen = document.createElement('canvas');
offscreen.width = 64;
offscreen.height = 32;
const octx = offscreen.getContext('2d');
const ctx = els.screen.getContext('2d');
let image = octx.createImageData(64, 32);

/* ----------------------------------------------------------- emulation -- */

function tick() {
  const wasm = app.wasm;
  if (!wasm || app.halted) return;
  wasm.rc_set_keys(keyMask());
  // 12 CHIP-8 instructions per frame == 1000 per-mille; the GB core maps that
  // to its own 70,224 cycles per frame.
  wasm.rc_tick(Math.round(SPEEDS[app.speed].cycles * (1000 / 12)));
  setBuzzer(wasm.rc_sound() > 0);
  if (wasm.rc_halted()) {
    app.halted = true;
    setBuzzer(false);
    showOverlay('Core halted', 'This ROM hit an unsupported opcode. Press ↺ to restart.');
  }
}

let last = 0;
let accumulator = 0;
let frameCount = 0;
let fpsSince = 0;

function loop(now) {
  if (!last) {
    last = now;
    fpsSince = now;
  }
  const delta = now - last;
  last = now;

  if (app.ready && !app.paused && !app.halted) {
    accumulator += Math.min(delta, 100);
    let steps = 0;
    while (accumulator >= FRAME_MS && steps < 5) {
      tick();
      accumulator -= FRAME_MS;
      steps += 1;
      if (app.halted) break;
    }
    if (steps === 5) accumulator = 0;
    render();
  }

  frameCount += 1;
  if (now - fpsSince >= 500) {
    els.fps.textContent = String(Math.round((frameCount * 1000) / (now - fpsSince)));
    frameCount = 0;
    fpsSince = now;
  }
  requestAnimationFrame(loop);
}

/* --------------------------------------------------------------- state -- */

function toast(message) {
  els.toast.textContent = message;
  els.toast.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => {
    els.toast.hidden = true;
  }, 1800);
}

function showOverlay(title, body) {
  els.overlay.innerHTML = `<b>${title}</b><span>${body}</span>`;
  els.overlay.hidden = false;
}

function hideOverlay() {
  els.overlay.hidden = true;
}

function bytesToBase64(bytes) {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function base64ToBytes(text) {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

function stateKey() {
  return `rustcade:state:${app.romId}`;
}

function applySystem() {
  const gb = app.system === 'gb';
  document.body.dataset.system = app.system;
  els.pad.hidden = gb;
  if (gb) els.keypad.hidden = true;
  els.hz.textContent = gb ? '4.19' : '720';
  els.hzUnit.textContent = gb ? 'MHz' : 'Hz';
  els.sysline.textContent = gb
    ? 'Rust DMG core · CPU · PPU · MBC1'
    : 'Rust CHIP-8 core · SCHIP / XO-CHIP';
  if (app.wasm) {
    els.screen.style.aspectRatio = `${app.wasm.rc_width()} / ${app.wasm.rc_height()}`;
  }
}

function updateSpeed() {
  els.speed.textContent = SPEEDS[app.speed].label;
}

function markActiveChip() {
  els.roms.querySelectorAll('.chip').forEach((chip) => {
    chip.classList.toggle('active', chip.dataset.id === app.romId);
  });
}

function installRom(id, title, bytes, hint, kind) {
  if (!app.wasm) {
    toast('Emulator core not loaded');
    return false;
  }
  const max = app.wasm.rc_rom_max(kind || 0);
  if (bytes.length === 0 || bytes.length > max) {
    toast(`ROM must be 1–${max} bytes`);
    return false;
  }
  const ptr = app.wasm.rc_reserve_io(bytes.length);
  if (!ptr) {
    toast('Could not allocate a ROM buffer');
    return false;
  }
  new Uint8Array(app.memory.buffer, ptr, bytes.length).set(bytes);
  const result = app.wasm.rc_load_rom(bytes.length, kind || 0);
  if (result !== 0) {
    toast(result === -3 ? 'ROM is too large for this core' : 'The core rejected this ROM');
    return false;
  }
  app.system = app.wasm.rc_system() === 2 ? 'gb' : 'chip8';
  pressed.clear();
  setBuzzer(false);
  app.romId = id;
  app.romName = title;
  app.ready = true;
  app.halted = false;
  app.paused = false;
  accumulator = 0;
  els.pause.textContent = '❚❚';
  hideOverlay();
  els.hint.textContent = hint || 'Touch controls map to the selected system';
  localStorage.setItem('rustcade:last', id);
  applySystem();
  markActiveChip();
  render();
  return true;
}

async function loadGame(game) {
  try {
    const response = await fetch(`roms/${game.file}`);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const kind = game.system === 'gb' ? 2 : game.system === 'chip8' ? 1 : 0;
    if (installRom(game.id, game.title, bytes, `${game.by} — ${game.hint}`, kind)) {
      toast(`${game.title} loaded`);
    }
  } catch (error) {
    toast(`Could not load ${game.title}`);
  }
}

async function loadLocalFile(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const title = file.name.replace(/\.[^.]+$/, '');
  installRom(`local:${file.name}`, title, bytes, file.name, 0);
}

function renderChips() {
  els.roms.innerHTML = '';
  for (const game of app.games) {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'chip';
    chip.dataset.id = game.id;
    chip.dataset.system = game.system || 'chip8';
    chip.textContent = game.title;
    chip.title = `${game.by} — ${game.hint}`;
    chip.addEventListener('click', () => {
      unlockAudio();
      loadGame(game);
    });
    els.roms.appendChild(chip);
  }
  markActiveChip();
}

function togglePause() {
  if (!app.ready) return;
  app.paused = !app.paused;
  els.pause.textContent = app.paused ? '▶' : '❚❚';
  if (app.paused) {
    setBuzzer(false);
    showOverlay('Paused', 'Press ▶ to continue.');
  } else {
    hideOverlay();
    accumulator = 0;
  }
}

/* --------------------------------------------------------------- boot --- */

function wireControls() {
  els.pause.addEventListener('click', () => {
    unlockAudio();
    togglePause();
  });
  els.reset.addEventListener('click', () => {
    if (!app.wasm) return;
    app.wasm.rc_reset();
    app.halted = false;
    app.paused = false;
    pressed.clear();
    setBuzzer(false);
    els.pause.textContent = '❚❚';
    hideOverlay();
    render();
  });
  els.speed.addEventListener('click', () => {
    app.speed = (app.speed + 1) % SPEEDS.length;
    updateSpeed();
    toast(`Speed ${SPEEDS[app.speed].label}`);
  });
  els.pad.addEventListener('click', () => {
    els.keypad.hidden = !els.keypad.hidden;
    if (!els.keypad.hidden) toast('Hex keypad — for games needing other keys');
  });
  els.save.addEventListener('click', () => {
    if (!app.ready) return;
    const length = app.wasm.rc_save_state();
    const bytes = new Uint8Array(app.memory.buffer, app.wasm.rc_state_ptr(), length).slice();
    try {
      localStorage.setItem(stateKey(), bytesToBase64(bytes));
      toast('State saved');
    } catch {
      toast('Save failed: storage full');
    }
  });
  els.load.addEventListener('click', () => {
    if (!app.ready) return;
    const stored = localStorage.getItem(stateKey());
    if (!stored) {
      toast('No saved state for this game');
      return;
    }
    const bytes = base64ToBytes(stored);
    if (bytes.length > app.wasm.rc_state_cap()) {
      toast('Saved state is not compatible');
      return;
    }
    new Uint8Array(app.memory.buffer, app.wasm.rc_state_ptr(), bytes.length).set(bytes);
    if (app.wasm.rc_load_state(bytes.length) === 0) {
      app.halted = false;
      hideOverlay();
      render();
      toast('State loaded');
    } else {
      toast('Saved state is not compatible');
    }
  });
  els.full.addEventListener('click', async () => {
    try {
      if (!document.fullscreenElement) {
        await els.app.requestFullscreen();
        if (screen.orientation && screen.orientation.lock) {
          screen.orientation.lock('landscape').catch(() => {});
        }
      } else {
        await document.exitFullscreen();
      }
    } catch {}
    setTimeout(render, 150);
  });
  els.file.addEventListener('change', async (event) => {
    const file = event.target.files && event.target.files[0];
    if (file) await loadLocalFile(file);
    event.target.value = '';
  });
  window.addEventListener('dragover', (event) => event.preventDefault());
  window.addEventListener('drop', async (event) => {
    event.preventDefault();
    const file = event.dataTransfer && event.dataTransfer.files[0];
    if (file) await loadLocalFile(file);
  });
  window.addEventListener('resize', () => render());
  window.addEventListener('visibilitychange', () => {
    if (document.hidden) {
      setBuzzer(false);
      pressed.clear();
    } else {
      last = 0;
    }
  });
}

async function boot() {
  wirePad();
  wireKeyboard();
  wireControls();
  updateSpeed();
  applySystem();
  requestAnimationFrame(loop);

  try {
    const response = await fetch('emulator.wasm');
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const module = await WebAssembly.instantiate(await response.arrayBuffer(), {});
    app.wasm = module.instance.exports;
    app.memory = app.wasm.memory;
    if (!app.memory) throw new Error('missing memory export');
  } catch (error) {
    showOverlay(
      'Core missing',
      'Build web/emulator.wasm with ./scripts/build.sh, then serve this folder over HTTP.'
    );
    return;
  }

  try {
    const response = await fetch('roms/index.json');
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    app.games = await response.json();
    renderChips();
  } catch {
    app.games = [];
    els.hint.textContent = 'No bundled games found — use “+ rom” to open a ROM file.';
  }

  const remembered = localStorage.getItem('rustcade:last');
  const pick = app.games.find((game) => game.id === remembered) || app.games[0];
  if (pick) {
    await loadGame(pick);
  } else {
    els.badge.textContent = 'No ROM loaded';
  }
  render();
}

boot();
