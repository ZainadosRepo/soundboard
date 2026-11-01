// Trim, fades and zoom of the waveform editor. Pure: no DOM, tested with node --test.

/// Shortest playable part a trim leaves.
export const MINIMUM_LENGTH = 0.1;
/// Shortest span the editor zooms to.
export const MINIMUM_VISIBLE = 0.05;
export const ZOOM_FACTOR = 2;

const clamp = (value, low, high) => Math.min(Math.max(value, low), high);

/// A span of the sound in seconds, always ordered.
export function range(a, b) {
  return { start: Math.min(a, b), end: Math.max(a, b) };
}

/// Edits of a tile's trim and fades that keep them consistent: at least MINIMUM_LENGTH stays
/// playable and each fade fits in half the playable part. Settings are the tile's `player`.
export class TrimEditor {
  constructor(duration) {
    this.duration = duration;
  }

  /// Where playback stops, in seconds into the file.
  end(s) {
    return this.duration - s.trimEnd;
  }

  maximumFade(s) {
    return Math.max(this.end(s) - s.trimStart, 0) / 2;
  }

  fittingFades(s) {
    const limit = this.maximumFade(s);
    return { ...s, fadeInTime: Math.min(s.fadeInTime, limit), fadeOutTime: Math.min(s.fadeOutTime, limit) };
  }

  /// Moves the start point to `time` seconds into the file.
  settingStart(time, s) {
    const latest = this.duration - s.trimEnd - MINIMUM_LENGTH;
    return this.fittingFades({ ...s, trimStart: clamp(time, 0, Math.max(latest, 0)) });
  }

  /// Moves the end point to `time` seconds into the file.
  settingEnd(time, s) {
    const earliest = s.trimStart + MINIMUM_LENGTH;
    return this.fittingFades({ ...s, trimEnd: this.duration - clamp(time, Math.min(earliest, this.duration), this.duration) });
  }

  /// Ends the fade-in at `time` seconds into the file.
  settingFadeInEnd(time, s) {
    return { ...s, fadeInTime: clamp(time - s.trimStart, 0, this.maximumFade(s)) };
  }

  /// Starts the fade-out at `time` seconds into the file.
  settingFadeOutStart(time, s) {
    return { ...s, fadeOutTime: clamp(this.end(s) - time, 0, this.maximumFade(s)) };
  }

  /// Keeps only the selection, without touching the sound file.
  cropping(selection, s) {
    const cropped = this.settingEnd(selection.end, this.settingStart(selection.start, s));
    return this.settingStart(selection.start, cropped);
  }

  /// Starts the sound at the selection and fades it in over the selection.
  fadingIn(selection, s) {
    return this.settingFadeInEnd(selection.end, this.settingStart(selection.start, s));
  }

  /// Ends the sound at the selection's end and fades it out over the selection.
  fadingOut(selection, s) {
    return this.settingFadeOutStart(selection.start, this.settingEnd(selection.end, s));
  }

  /// Removes trim and fades; the sound file was never changed.
  reverting(s) {
    return { ...s, trimStart: 0, trimEnd: 0, fadeInTime: 0, fadeOutTime: 0 };
  }
}

/// The part of a sound the editor shows, for zooming and scrolling.
export class Viewport {
  constructor(duration) {
    this.duration = Math.max(duration, 0);
    this.visible = range(0, this.duration);
  }

  get length() {
    return this.visible.end - this.visible.start;
  }

  zoomIn() {
    this.#zoomAround(this.length / ZOOM_FACTOR, (this.visible.start + this.visible.end) / 2);
  }

  zoomOut() {
    this.#zoomAround(this.length * ZOOM_FACTOR, (this.visible.start + this.visible.end) / 2);
  }

  zoomToFit() {
    this.visible = range(0, this.duration);
  }

  zoomTo(span) {
    this.#zoomAround(span.end - span.start, (span.start + span.end) / 2);
  }

  /// Zooms by `factor` (above 1 zooms in) keeping the time at `fraction` of the width where it
  /// is, as zooming around the pointer does.
  zoomBy(factor, fraction) {
    if (!(factor > 0) || !Number.isFinite(factor)) return;
    fraction = clamp(fraction, 0, 1);
    const anchor = this.visible.start + fraction * this.length;
    const length = this.#clamped(this.length / factor);
    this.#place(length, anchor - fraction * length);
  }

  /// Moves the visible span by `seconds`, staying inside the sound.
  scroll(seconds) {
    this.#place(this.length, this.visible.start + seconds);
  }

  /// Seconds into the sound at `x` in a view `width` pixels wide.
  timeAt(x, width) {
    if (width <= 0) return this.visible.start;
    return this.visible.start + clamp(x / width, 0, 1) * this.length;
  }

  /// Horizontal position of `time` in a view `width` pixels wide; may fall outside it.
  x(time, width) {
    if (this.length <= 0) return 0;
    return ((time - this.visible.start) / this.length) * width;
  }

  #zoomAround(length, center) {
    length = this.#clamped(length);
    this.#place(length, center - length / 2);
  }

  #clamped(length) {
    return clamp(length, Math.min(MINIMUM_VISIBLE, this.duration), this.duration);
  }

  #place(length, start) {
    start = clamp(start, 0, this.duration - length);
    this.visible = range(start, start + length);
  }
}

/// Regroups the peaks covering `start…end` (fractions of the sound) into `count` columns,
/// keeping extremes, so a long or zoomed waveform draws one bar per pixel column.
export function columns(peaks, start, end, count) {
  if (!peaks.length || count <= 0 || end <= start) return [];
  const first = peaks.length * clamp(start, 0, 1);
  const span = peaks.length * clamp(end, 0, 1) - first;
  const result = new Array(count);
  for (let column = 0; column < count; column++) {
    const lower = Math.min(Math.floor(first + (span * column) / count), peaks.length - 1);
    const upper = Math.min(Math.max(Math.floor(first + (span * (column + 1)) / count), lower + 1), peaks.length);
    let low = peaks[lower].low;
    let high = peaks[lower].high;
    for (let i = lower + 1; i < upper; i++) {
      if (peaks[i].low < low) low = peaks[i].low;
      if (peaks[i].high > high) high = peaks[i].high;
    }
    result[column] = { low, high };
  }
  return result;
}

/// "1,25 s"
export function seconds(value) {
  return `${(value ?? 0).toFixed(2).replace(".", ",")} s`;
}

/// Length once trimmed, as m:ss; "–:––" while unknown.
export function playLength(duration, player) {
  if (duration == null) return "–:––";
  const total = Math.round(Math.max(duration - player.trimStart - player.trimEnd, 0));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/// What a click or key does to a playing tile: ⇧ pauses, the command key (Ctrl on Windows) fades.
export function triggerModifier(event, mac) {
  if (event.shiftKey) return "pause";
  if (mac ? event.metaKey : event.ctrlKey) return "fade";
  return "none";
}

/// Tauri accelerator for a key press while recording the global shortcut, or an error when
/// no Ctrl, Alt or Command/Windows key is held.
export function accelerator(event) {
  const ignored = ["Control", "Shift", "Alt", "Meta", "OS"];
  if (ignored.includes(event.key)) return null;
  const parts = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Super");
  if (!event.ctrlKey && !event.altKey && !event.metaKey) return { error: "Ajoutez Ctrl, Alt ou la touche Windows/Commande." };
  const code = event.code || "";
  const key = code.startsWith("Key") ? code.slice(3) : code.startsWith("Digit") ? code.slice(5) : code;
  if (!key) return null;
  return { accelerator: [...parts, key].join("+") };
}
