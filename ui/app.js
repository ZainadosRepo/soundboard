import { TrimEditor, Viewport, columns, range, seconds, playLength, triggerModifier, accelerator } from "./editing.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const MAC = navigator.userAgent.includes("Mac");
const MOD = MAC ? "⌘" : "Ctrl";
const COLORS = ["purple", "pink", "red", "orange", "yellow", "green", "lightBlue", "darkBlue", "black"];
const COLOR_NAMES = {
  purple: "Violet",
  pink: "Rose",
  red: "Rouge",
  orange: "Orange",
  yellow: "Jaune",
  green: "Vert",
  lightBlue: "Bleu clair",
  darkBlue: "Bleu foncé",
  black: "Noir",
};
/// Pointer travel, in pixels, that turns a click into a drag.
const DRAG_THRESHOLD = 5;
const SCAN_STEP = 5;
const FINE_SCAN_STEP = 1;

const $ = (id) => document.getElementById(id);

let state = null;
let inspectorOpen = false;
/// While a tile is held or dragged, redraws wait: replacing its element would lose the pointer.
let holding = 0;
let renderPending = false;
let editor = null;
let renamingSet = null;
let errorShown = null;
const waveforms = new Map();

// ---------- helpers

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function button(label, className, onClick, title) {
  const b = el("button", className, label);
  b.type = "button";
  if (title) b.title = title;
  b.addEventListener("click", onClick);
  return b;
}

async function call(command, args) {
  try {
    return await invoke(command, args);
  } catch (error) {
    alertDialog("Un problème est survenu", String(error));
    return undefined;
  }
}

const isTyping = (target) => target instanceof HTMLElement && (target.matches("input:not([type=range]):not([type=checkbox]), textarea, select") || target.isContentEditable);

function tileById(id) {
  if (state?.selected?.id === id) return state.selected;
  return state?.set.tiles.find((t) => t.id === id);
}

/// The stored tile without the view-only fields, ready to send back.
function plain(tile) {
  const { state: _state, duration: _duration, ...rest } = tile;
  return structuredClone(rest);
}

function updateTile(id, change) {
  const tile = tileById(id);
  if (!tile) return;
  const edited = plain(tile);
  change(edited);
  call("update_tile", { tile: edited });
}

async function waveform(tile, editorBins) {
  const key = `${tile.fileId}:${editorBins ? "editor" : "board"}`;
  if (!waveforms.has(key)) {
    waveforms.set(
      key,
      invoke("waveform", { id: tile.id, editor: editorBins }).catch((e) => {
        waveforms.delete(key);
        throw e;
      }),
    );
  }
  return waveforms.get(key);
}

// ---------- menus and dialogs

function closeMenus() {
  $("menu").hidden = true;
  $("popover").hidden = true;
}

/// Items: { label, action, shortcut, disabled, danger } | "sep" | { colors: current, pick }.
function showMenu(x, y, items) {
  closeMenus();
  const menu = $("menu");
  menu.replaceChildren();
  for (const item of items) {
    if (item === "sep") {
      menu.append(el("div", "menu-sep"));
    } else if (item.colors) {
      const row = el("div", "swatches");
      for (const color of COLORS) {
        const swatch = button("", `swatch-btn color-${color}${item.colors === color ? " on" : ""}`, () => {
          closeMenus();
          item.pick(color);
        }, COLOR_NAMES[color]);
        row.append(swatch);
      }
      menu.append(row);
    } else {
      const b = button("", `menu-item${item.danger ? " danger" : ""}`, () => {
        closeMenus();
        item.action();
      });
      b.append(el("span", null, `${item.checked ? "✓ " : ""}${item.label}`));
      if (item.shortcut) b.append(el("span", "shortcut", item.shortcut));
      b.disabled = !!item.disabled;
      menu.append(b);
    }
  }
  menu.hidden = false;
  menu.style.left = `${Math.min(x, window.innerWidth - menu.offsetWidth - 8)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - menu.offsetHeight - 8)}px`;
}

function showPopover(anchor, content) {
  closeMenus();
  const pop = $("popover");
  pop.replaceChildren(content);
  pop.hidden = false;
  const box = anchor.getBoundingClientRect();
  pop.style.left = `${Math.max(8, Math.min(box.left, window.innerWidth - pop.offsetWidth - 8))}px`;
  pop.style.top = `${box.bottom + 6}px`;
}

/// Shows `body` with `buttons` ({ label, primary, danger, value }); resolves with the value.
function openDialog(body, buttons) {
  const dialog = $("dialog");
  return new Promise((resolve) => {
    const content = el("div", "dialog-body");
    content.append(...[body].flat());
    const foot = el("div", "dialog-foot");
    for (const b of buttons) {
      foot.append(
        button(b.label, `btn${b.primary ? " primary" : ""}${b.danger ? " danger" : ""}`, () => {
          dialog.close();
          resolve(b.value);
        }),
      );
    }
    dialog.replaceChildren(content, foot);
    dialog.onclose = () => resolve(undefined);
    dialog.showModal();
  });
}

function alertDialog(title, message) {
  return openDialog([el("h2", null, title), el("p", null, message)], [{ label: "OK", primary: true, value: true }]);
}

async function confirmDialog(title, message, action) {
  const ok = await openDialog([el("h2", null, title), el("p", null, message)], [
    { label: "Annuler", value: false },
    { label: action, danger: true, primary: true, value: true },
  ]);
  return ok === true;
}

// ---------- rendering

function render() {
  if (!state) return;
  if (holding) {
    renderPending = true;
    return;
  }
  renderSidebar();
  renderToolbar();
  renderBoard();
  renderInspector();
  if (editor) renderEditor();
  showErrorIfNew();
}

function showErrorIfNew() {
  if (!state.error || state.error === errorShown || $("dialog").open) return;
  errorShown = state.error;
  alertDialog("Un problème est survenu", state.error).then(() => {
    errorShown = null;
    call("dismiss_error");
  });
}

function renderSidebar() {
  const list = $("sets");
  list.replaceChildren();
  for (const set of state.sets) {
    const item = el("li", `set${set.id === state.set.id ? " selected" : ""}`);
    item.setAttribute("role", "option");
    item.append(el("span", null, set.smart ? "⚙" : "▦"));
    if (renamingSet === set.id) {
      const input = el("input");
      input.value = set.title;
      let done = false;
      const commit = (save) => {
        if (done) return;
        done = true;
        renamingSet = null;
        if (save) call("rename_set", { id: set.id, title: input.value });
        else render();
      };
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") commit(true);
        if (e.key === "Escape") commit(false);
        e.stopPropagation();
      });
      input.addEventListener("blur", () => commit(true));
      item.append(input);
      queueMicrotask(() => input.select());
    } else {
      item.append(el("span", "name", set.title), el("span", "count", String(set.count)));
    }
    item.addEventListener("click", () => set.id !== state.set.id && call("select_set", { id: set.id }));
    item.addEventListener("dblclick", () => {
      renamingSet = set.id;
      render();
    });
    item.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      showMenu(e.clientX, e.clientY, [
        { label: "Renommer", action: () => ((renamingSet = set.id), render()) },
        ...(set.smart ? [{ label: "Modifier les règles…", action: () => editRules(set.id) }] : []),
        "sep",
        { label: "Supprimer…", danger: true, disabled: !state.canDeleteSet, action: () => deleteSet(set) },
      ]);
    });
    list.append(item);
  }
}

async function deleteSet(set) {
  const ok = await confirmDialog(`Supprimer « ${set.title} » ?`, "Ses tuiles et les copies de leurs sons seront supprimées. Cette action est irréversible.", "Supprimer le set");
  if (ok) call("delete_set", { id: set.id });
}

function renderToolbar() {
  $("set-title").textContent = state.set.title;
  const list = state.set.displayMode === "list";
  $("mode-grid").classList.toggle("on", !list);
  $("mode-list").classList.toggle("on", list);
  const master = state.master;
  $("master-a").classList.toggle("on", !master.usesVolumeB);
  $("master-b").classList.toggle("on", master.usesVolumeB);
  const active = master.usesVolumeB ? master.volumeB : master.volumeA;
  if (document.activeElement !== $("master-volume")) $("master-volume").value = active;
  $("master-value").textContent = `${Math.round(active * 100)} %`;
  $("master-mute").textContent = master.muted ? "🔇" : "🔊";
  $("master-mute").classList.toggle("on", master.muted);
  $("toggle-inspector").classList.toggle("on", inspectorOpen);
  $("add-sounds").disabled = !!state.set.smartRules;
}

function renderBoard() {
  const board = $("board");
  const set = state.set;
  board.replaceChildren();
  if (set.smartRules && !set.tiles.length) {
    const empty = el("div", "empty");
    empty.append(el("strong", null, "Aucune tuile ne correspond"), el("span", null, "Ce smart set montre les tuiles des autres sets qui suivent ses règles."), button("Modifier les règles…", "btn", () => editRules(set.id)));
    board.append(empty);
    return;
  }
  if (!set.tiles.length) {
    const empty = el("div", "empty");
    empty.append(
      el("strong", null, "Déposez des fichiers audio ici"),
      el("span", null, "Chaque fichier devient une tuile : cliquez dessus pour la lancer ou l’arrêter. Déposez un set pour l’ouvrir."),
      button("Ajouter des sons…", "btn", () => call("choose_sounds")),
    );
    board.append(empty);
    return;
  }
  const list = set.displayMode === "list";
  const container = el("div", list ? "list" : "grid");
  set.tiles.forEach((tile, index) => container.append(list ? listRow(tile, index + 1) : gridTile(tile)));
  board.append(container);
}

function keyBadges(tile) {
  const keys = [];
  if (tile.hotkey) keys.push(el("span", "key", tile.hotkey.toUpperCase()));
  if (tile.midiTrigger) keys.push(el("span", "key", `♪ ${noteName(tile.midiTrigger.note)}`));
  return keys;
}

function stateSymbol(tile) {
  return tile.state === "playing" ? "🔊" : tile.state === "paused" ? "⏸" : "";
}

function gridTile(tile) {
  const node = el("div", `tile color-${tile.color}`);
  node.dataset.id = tile.id;
  node.classList.toggle("disabled", !tile.isEnabled);
  node.classList.toggle("playing", tile.state === "playing");
  node.classList.toggle("selected", state.selected?.id === tile.id);
  node.title = `Clic : lancer/arrêter · Maj-clic : pause · ${MOD}-clic : fondu de sortie`;
  const keys = el("div", "keys");
  keys.append(...keyBadges(tile));
  node.append(keys, el("div", "state", stateSymbol(tile)));
  if (tile.icon) node.append(el("div", "icon", tile.icon));
  node.append(el("div", "title", tile.title));
  const badges = el("div", "badges");
  if (tile.player.loops) badges.append(el("span", null, "🔁"));
  if (tile.solo) badges.append(el("span", null, "Ⓢ"));
  if (tile.holdToPlay) badges.append(el("span", null, "✋"));
  node.append(badges);
  trigger(node, tile);
  return node;
}

function listRow(tile, number) {
  const node = el("div", "row");
  node.dataset.id = tile.id;
  node.classList.toggle("disabled", !tile.isEnabled);
  node.classList.toggle("playing", tile.state === "playing");
  node.classList.toggle("selected", state.selected?.id === tile.id);
  node.classList.toggle("played", tile.lastPlayed != null);
  const swatch = el("span", `swatch color-${tile.color}`);
  node.append(el("span", "number", String(number)), swatch, el("span", "icon", tile.icon || " "), el("span", "title", tile.title));
  if (tile.player.loops) node.append(el("span", null, "🔁"));
  node.append(...keyBadges(tile), el("span", "state", stateSymbol(tile)), el("span", "length", playLength(tile.duration, tile.player)));
  const mark = el("span", `mark${tile.lastPlayed != null ? " done" : ""}`, tile.lastPlayed != null ? "●" : "○");
  mark.title = tile.lastPlayed != null ? "Déjà joué" : "Pas encore joué";
  node.append(mark);
  trigger(node, tile);
  return node;
}

/// Clicking plays the tile on release; Hold to Play tiles start on press and stop on release.
/// Moving a few pixels before releasing drags the tile onto another place instead.
function trigger(node, tile) {
  node.addEventListener("pointerdown", (down) => {
    if (down.button !== 0) return;
    node.setPointerCapture(down.pointerId);
    holding++;
    const modifier = triggerModifier(down, MAC);
    const listMode = state.set.displayMode === "list";
    const startsOnPress = tile.isEnabled && tile.holdToPlay && !listMode;
    let dragging = false;
    node.classList.add("pressed");
    if (startsOnPress) call("press", { id: tile.id, modifier });
    const move = (e) => {
      const dx = e.clientX - down.clientX;
      const dy = e.clientY - down.clientY;
      if (!dragging && Math.hypot(dx, dy) <= DRAG_THRESHOLD) return;
      dragging = true;
      node.classList.add("dragging");
      node.style.transform = `translate(${dx}px, ${dy}px)`;
    };
    const end = (e) => {
      node.removeEventListener("pointermove", move);
      node.removeEventListener("pointerup", end);
      node.removeEventListener("pointercancel", end);
      if (dragging && e.type === "pointerup") dropOnNearest(tile, e.clientX, e.clientY);
      else if (!dragging && !startsOnPress && e.type === "pointerup") call("press", { id: tile.id, modifier });
      if (startsOnPress) call("release", { id: tile.id });
      holding--;
      render();
    };
    node.addEventListener("pointermove", move);
    node.addEventListener("pointerup", end);
    node.addEventListener("pointercancel", end);
  });
  node.addEventListener("dblclick", () => openInspector());
  node.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    call("select_tile", { id: tile.id });
    tileMenu(tile, e.clientX, e.clientY);
  });
}

/// Drops onto the tile laid out closest to the release point; dropping nearest the dragged
/// tile's own place leaves the order unchanged.
function dropOnNearest(tile, x, y) {
  let best = null;
  for (const node of $("board").querySelectorAll("[data-id]")) {
    const box = node.getBoundingClientRect();
    if (node.dataset.id === tile.id) {
      // Its original place, before the drag moved it.
      const shift = new DOMMatrixReadOnly(getComputedStyle(node).transform);
      box.x -= shift.m41;
      box.y -= shift.m42;
    }
    const distance = Math.hypot(x - (box.x + box.width / 2), y - (box.y + box.height / 2));
    if (!best || distance < best.distance) best = { id: node.dataset.id, distance };
  }
  if (best && best.id !== tile.id) call("move_tile", { id: tile.id, onto: best.id });
}

function tileMenu(tile, x, y) {
  const smart = !!state.set.smartRules;
  const toggle = (label, read, write) => ({ label, checked: read(tile), action: () => updateTile(tile.id, (t) => write(t, !read(t))) });
  showMenu(x, y, [
    { label: "Modifier…", action: () => openInspector() },
    { label: "Ouvrir l’éditeur de forme d’onde", shortcut: `${MOD} + Maj + E`, action: () => openEditor(tile.id) },
    "sep",
    toggle("Boucle", (t) => t.player.loops, (t, v) => (t.player.loops = v)),
    toggle("Favori", (t) => t.isFavorite, (t, v) => (t.isFavorite = v)),
    toggle("Solo", (t) => t.solo, (t, v) => (t.solo = v)),
    toggle("Maintenir pour jouer", (t) => t.holdToPlay, (t, v) => (t.holdToPlay = v)),
    toggle("Activée", (t) => t.isEnabled, (t, v) => (t.isEnabled = v)),
    { colors: tile.color, pick: (color) => updateTile(tile.id, (t) => (t.color = color)) },
    ...(smart ? [] : ["sep", { label: "Supprimer…", danger: true, action: () => deleteTile(tile) }]),
  ]);
}

async function deleteTile(tile) {
  const ok = await confirmDialog(`Supprimer « ${tile.title} » ?`, "La copie de son son dans ce set sera supprimée. Cette action est irréversible.", "Supprimer la tuile");
  if (ok) call("delete_tile", { id: tile.id });
}

function noteName(note) {
  const names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
  return `${names[note % 12]}${Math.floor(note / 12) - 2}`;
}

// ---------- inspector

let inspectorTile = null;

function openInspector() {
  inspectorOpen = true;
  render();
}

/// Fields are built once per tile and then only refreshed, so typing is never interrupted.
function renderInspector() {
  const panel = $("inspector");
  panel.hidden = !inspectorOpen;
  if (!inspectorOpen) return;
  const tile = state.selected;
  if (!tile) {
    inspectorTile = null;
    panel.replaceChildren(el("div", "empty", "Aucune tuile sélectionnée"));
    return;
  }
  if (inspectorTile !== tile.id) {
    inspectorTile = tile.id;
    buildInspector(panel, tile);
  }
  refreshInspector(panel, tile);
}

function bindText(panel, label, field, multiline, placeholder) {
  const row = el("label", "field");
  const input = el(multiline ? "textarea" : "input");
  input.dataset.field = field;
  if (placeholder) input.placeholder = placeholder;
  input.addEventListener("input", () => updateTile(state.selected.id, (t) => (t[field] = input.value)));
  input.addEventListener("keydown", (e) => e.stopPropagation());
  row.append(el("span", null, label), input);
  panel.append(row);
}

function bindCheck(panel, label, read, write) {
  const row = el("label", "check");
  const input = el("input");
  input.type = "checkbox";
  input.dataset.check = label;
  input.addEventListener("change", () => updateTile(state.selected.id, (t) => write(t, input.checked)));
  row.append(input, el("span", null, label));
  row.read = read;
  panel.append(row);
}

function bindVolume(panel, label, key) {
  const row = el("label", "field");
  const input = el("input");
  input.type = "range";
  input.min = 0;
  input.max = 1;
  input.step = 0.01;
  input.dataset.volume = key;
  const out = el("output");
  out.dataset.volumeOut = key;
  input.addEventListener("input", () => updateTile(state.selected.id, (t) => (t.player[key] = Number(input.value))));
  row.append(el("span", null, label), input, out);
  panel.append(row);
}

function buildInspector(panel, tile) {
  panel.replaceChildren();
  panel.append(el("h3", null, "Tuile"));
  bindText(panel, "Titre", "title");
  bindText(panel, "Icône", "icon", false, "Emoji");
  const colorRow = el("label", "field");
  const color = el("select");
  color.dataset.select = "color";
  for (const c of COLORS) {
    const option = el("option", null, COLOR_NAMES[c]);
    option.value = c;
    color.append(option);
  }
  color.addEventListener("change", () => updateTile(state.selected.id, (t) => (t.color = color.value)));
  colorRow.append(el("span", null, "Couleur"), color);
  panel.append(colorRow);
  bindText(panel, "Notes", "notes", true);

  panel.append(el("h3", null, "Clavier"));
  const keyRow = el("div", "field");
  const key = el("input", "hotkey-input");
  key.placeholder = "Aucune";
  key.dataset.hotkey = "1";
  key.addEventListener("keydown", (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Backspace" || e.key === "Delete") call("set_hotkey", { id: state.selected.id, key: null });
    else if (e.key.length === 1) call("set_hotkey", { id: state.selected.id, key: tileKey(e) });
  });
  keyRow.append(el("span", null, "Touche"), key, button("Effacer", "btn", () => call("set_hotkey", { id: state.selected.id, key: null })));
  panel.append(keyRow, el("p", "help", "Une lettre ou un chiffre lance cette tuile quand son set est affiché ; Maj la met en pause. Donner la touche à une autre tuile du set la retire à celle-ci."));

  panel.append(el("h3", null, "MIDI"));
  const midi = el("div");
  midi.dataset.midi = "1";
  panel.append(midi);

  panel.append(el("h3", null, "Lecture"));
  bindCheck(panel, "Activée", (t) => t.isEnabled, (t, v) => (t.isEnabled = v));
  bindCheck(panel, "Boucle", (t) => t.player.loops, (t, v) => (t.player.loops = v));
  bindCheck(panel, "Favori", (t) => t.isFavorite, (t, v) => (t.isFavorite = v));
  bindCheck(panel, "Solo", (t) => t.solo, (t, v) => (t.solo = v));
  bindCheck(panel, "Maintenir pour jouer", (t) => t.holdToPlay, (t, v) => (t.holdToPlay = v));

  panel.append(el("h3", null, "Volume"));
  const abRow = el("div", "field");
  const ab = el("div", "segmented");
  ab.append(
    button("Volume A", "", () => updateTile(state.selected.id, (t) => (t.player.usesVolumeB = false))),
    button("Volume B", "", () => updateTile(state.selected.id, (t) => (t.player.usesVolumeB = true))),
  );
  ab.dataset.ab = "1";
  abRow.append(el("span", null, "Joue au"), ab);
  panel.append(abRow);
  bindVolume(panel, "Volume A", "volumeA");
  bindVolume(panel, "Volume B", "volumeB");
  bindCheck(panel, "Muet", (t) => t.player.muted, (t, v) => (t.player.muted = v));

  panel.append(el("h3", null, "Découpe et fondus"));
  const trim = el("div");
  trim.dataset.trim = "1";
  panel.append(trim);

  const del = el("div");
  del.dataset.delete = "1";
  panel.append(del);
}

function refreshInspector(panel, tile) {
  for (const input of panel.querySelectorAll("[data-field]")) {
    if (document.activeElement !== input) input.value = tile[input.dataset.field] ?? "";
  }
  const color = panel.querySelector("[data-select=color]");
  if (document.activeElement !== color) color.value = tile.color;
  panel.querySelector("[data-hotkey]").value = tile.hotkey?.toUpperCase() ?? "";
  for (const row of panel.querySelectorAll(".check")) row.querySelector("input").checked = row.read(tile);
  for (const input of panel.querySelectorAll("[data-volume]")) {
    if (document.activeElement !== input) input.value = tile.player[input.dataset.volume];
  }
  for (const out of panel.querySelectorAll("[data-volume-out]")) out.textContent = `${Math.round(tile.player[out.dataset.volumeOut] * 100)} %`;
  const [a, b] = panel.querySelector("[data-ab]").children;
  a.classList.toggle("on", !tile.player.usesVolumeB);
  b.classList.toggle("on", tile.player.usesVolumeB);

  const midi = panel.querySelector("[data-midi]");
  midi.replaceChildren();
  const midiRow = el("div", "field");
  midiRow.append(el("span", null, "Note"));
  if (state.midiLearning === tile.id) {
    midiRow.append(el("span", "learning", "Jouez une note…"), button("Annuler", "btn", () => call("midi_learn", { id: null })));
  } else {
    midiRow.append(el("span", null, tile.midiTrigger ? `${noteName(tile.midiTrigger.note)} · canal ${tile.midiTrigger.channel}` : "Aucune"));
    midiRow.append(button("Apprendre", "btn", () => call("midi_learn", { id: tile.id }), `Apprendre (${MOD} + L)`));
    if (tile.midiTrigger) midiRow.append(button("Effacer", "btn", () => call("clear_midi", { id: tile.id })));
  }
  const sources = state.midiSources === 0 ? "Aucune source MIDI connectée." : state.midiSources === 1 ? "1 source MIDI connectée." : `${state.midiSources} sources MIDI connectées.`;
  midi.append(midiRow, el("p", "help", `Une note jouée sur un contrôleur lance cette tuile quand son set est affiché ; la vélocité règle son volume. ${sources}`));

  const trim = panel.querySelector("[data-trim]");
  trim.replaceChildren();
  if (tile.duration == null) {
    trim.append(el("p", "help", "Lecture du son…"));
  } else {
    const wave = el("div", "wave");
    const canvas = el("canvas");
    wave.append(canvas);
    const readouts = el("div", "readouts");
    const p = tile.player;
    for (const [label, value] of [
      ["Début", p.trimStart],
      ["Fin", tile.duration - p.trimEnd],
      ["Fondu d’entrée", p.fadeInTime],
      ["Fondu de sortie", p.fadeOutTime],
    ]) {
      readouts.append(el("span", null, label), el("span", null, seconds(value)));
    }
    trim.append(
      wave,
      button("Ouvrir l’éditeur complet…", "btn wide", () => openEditor(tile.id)),
      readouts,
      el("p", "help", `Faites glisser les barres pour découper et les points jaunes pour régler les fondus. Durée du son : ${seconds(tile.duration)}. La découpe et la boucle s’appliquent au prochain lancement.`),
    );
    const view = { viewport: new Viewport(tile.duration), draft: null, selection: null };
    const draw = (peaks) => drawWave(canvas, { peaks, tile, view });
    draw(null);
    waveform(tile, false).then(draw, () => {});
    handleDrags(canvas, tile, view, draw, () => {});
  }
  const del = panel.querySelector("[data-delete]");
  del.replaceChildren();
  if (!state.set.smartRules) del.append(button("Supprimer la tuile…", "btn danger wide", () => deleteTile(tile)));
}

// ---------- waveform drawing and handles

function canvasSize(canvas) {
  const ratio = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.floor(canvas.clientWidth * ratio));
  const height = Math.max(1, Math.floor(canvas.clientHeight * ratio));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  return { width, height, ratio };
}

/// Draws the zoomed waveform with the kept part, selection, trim dimming, fade envelope,
/// handles and playhead.
function drawWave(canvas, { peaks, tile, view, playhead, handles = true }) {
  const { width, height, ratio } = canvasSize(canvas);
  const ctx = canvas.getContext("2d");
  const style = getComputedStyle(document.documentElement);
  ctx.clearRect(0, 0, width, height);
  const duration = tile.duration;
  const player = view.draft ?? tile.player;
  const vp = view.viewport;
  const x = (time) => vp.x(time, width);
  if (!peaks) {
    ctx.fillStyle = style.getPropertyValue("--muted");
    ctx.font = `${12 * ratio}px system-ui`;
    ctx.fillText("Chargement…", 10 * ratio, height / 2);
  } else {
    const bars = columns(peaks, vp.visible.start / duration, vp.visible.end / duration, width);
    const keptStart = x(player.trimStart);
    const keptEnd = x(duration - player.trimEnd);
    const mid = height / 2;
    bars.forEach((bar, i) => {
      ctx.fillStyle = i >= keptStart && i <= keptEnd ? style.getPropertyValue("--accent") : style.getPropertyValue("--faint");
      const top = mid - Math.max(bar.high, 0) * mid;
      const bottom = mid - Math.min(bar.low, 0) * mid;
      ctx.fillRect(i, top, 1, Math.max(bottom - top, 1));
    });
  }
  if (view.selection) {
    ctx.fillStyle = "rgba(124, 108, 255, 0.2)";
    const from = x(view.selection.start);
    ctx.fillRect(from, 0, Math.max(x(view.selection.end) - from, 1), height);
  }
  const startX = x(player.trimStart);
  const endX = x(duration - player.trimEnd);
  ctx.fillStyle = "rgba(0, 0, 0, 0.3)";
  ctx.fillRect(0, 0, Math.max(startX, 0), height);
  ctx.fillRect(endX, 0, Math.max(width - endX, 0), height);
  ctx.strokeStyle = style.getPropertyValue("--yellow");
  ctx.lineWidth = 1.5 * ratio;
  ctx.beginPath();
  ctx.moveTo(startX, height);
  ctx.lineTo(x(player.trimStart + player.fadeInTime), 0);
  ctx.lineTo(x(duration - player.trimEnd - player.fadeOutTime), 0);
  ctx.lineTo(endX, height);
  ctx.stroke();
  if (handles) {
    ctx.fillStyle = "#fff";
    ctx.fillRect(startX - ratio, 0, 2 * ratio, height);
    ctx.fillRect(endX - ratio, 0, 2 * ratio, height);
    ctx.fillStyle = style.getPropertyValue("--yellow");
    for (const time of [player.trimStart + player.fadeInTime, duration - player.trimEnd - player.fadeOutTime]) {
      ctx.beginPath();
      ctx.arc(x(time), 9 * ratio, 5.5 * ratio, 0, Math.PI * 2);
      ctx.fill();
    }
  }
  if (playhead != null) {
    ctx.fillStyle = "#fff";
    ctx.fillRect(x(playhead) - ratio / 2, 0, ratio, height);
  }
}

/// Handles: trim bars (full height) and fade dots (along the top). A drag edits a draft and
/// commits once on release; elsewhere on the canvas `otherwise` takes the pointer.
function handleDrags(canvas, tile, view, redraw, commit, otherwise) {
  const handlesAt = () => {
    const p = view.draft ?? tile.player;
    const d = tile.duration;
    return [
      { name: "Début", bar: true, time: p.trimStart, edit: (e, t, s) => e.settingStart(t, s) },
      { name: "Fin", bar: true, time: d - p.trimEnd, edit: (e, t, s) => e.settingEnd(t, s) },
      { name: "Fondu d’entrée", bar: false, time: p.trimStart + p.fadeInTime, edit: (e, t, s) => e.settingFadeInEnd(t, s) },
      { name: "Fondu de sortie", bar: false, time: d - p.trimEnd - p.fadeOutTime, edit: (e, t, s) => e.settingFadeOutStart(t, s) },
    ];
  };
  const hit = (e) => {
    const box = canvas.getBoundingClientRect();
    const px = e.clientX - box.left;
    const py = e.clientY - box.top;
    // Dots first: they sit on top of the bars at the ends of a fade.
    const ordered = handlesAt().sort((a, b) => a.bar - b.bar);
    return ordered.find((h) => {
      const hx = view.viewport.x(h.time, box.width);
      return h.bar ? Math.abs(px - hx) <= 6 : Math.hypot(px - hx, py - 9) <= 9;
    });
  };
  canvas.addEventListener("pointermove", (e) => {
    if (e.buttons) return;
    const h = hit(e);
    canvas.style.cursor = h ? (h.bar ? "ew-resize" : "pointer") : otherwise ? "text" : "default";
    canvas.title = h ? h.name : "";
  });
  canvas.addEventListener("pointerdown", (down) => {
    if (down.button !== 0) return;
    const handle = hit(down);
    if (!handle) {
      otherwise?.(down);
      return;
    }
    canvas.setPointerCapture(down.pointerId);
    holding++;
    const box = canvas.getBoundingClientRect();
    const editorLogic = new TrimEditor(tile.duration);
    const original = structuredClone(tile.player);
    // The handle moves by the pointer's travel from where it was grabbed, so grabbing it off
    // its centre line doesn't make it jump.
    const grabX = view.viewport.x(handle.time, box.width);
    let moved = false;
    const move = (e) => {
      moved = true;
      const time = view.viewport.timeAt(grabX + e.clientX - down.clientX, box.width);
      view.draft = handle.edit(editorLogic, time, original);
      redraw();
    };
    const up = () => {
      canvas.removeEventListener("pointermove", move);
      canvas.removeEventListener("pointerup", up);
      canvas.removeEventListener("pointercancel", up);
      const draft = view.draft;
      view.draft = null;
      if (moved && draft) {
        commit(draft, handle.name, original);
        updateTile(tile.id, (t) => (t.player = draft));
      }
      holding--;
      redraw();
      render();
    };
    canvas.addEventListener("pointermove", move);
    canvas.addEventListener("pointerup", up);
    canvas.addEventListener("pointercancel", up);
  });
}

// ---------- waveform editor

async function openEditor(id) {
  const tile = tileById(id);
  if (!tile) return;
  if (tile.duration == null) {
    alertDialog("Éditeur indisponible", "La durée du son n’est pas encore connue, ou le fichier est illisible.");
    return;
  }
  closeMenus();
  editor = { id, view: { viewport: new Viewport(tile.duration), draft: null, selection: null }, undo: [], redo: [], peaks: null, built: false };
  $("editor").hidden = false;
  renderEditor();
  try {
    editor.peaks = await waveform(tile, true);
  } catch (error) {
    alertDialog("Forme d’onde indisponible", String(error));
  }
  redrawEditor();
}

function closeEditor() {
  if (!editor) return;
  if (state.previewing) call("stop_preview");
  editor = null;
  $("editor").hidden = true;
  $("editor").replaceChildren();
}

function editorTile() {
  return editor && tileById(editor.id);
}

/// One undoable edit: Ctrl/⌘ + Z restores the previous settings.
function applyEdit(settings, name, previous) {
  const tile = editorTile();
  if (!tile) return;
  const before = previous ?? structuredClone(tile.player);
  editor.undo.push({ before, after: settings, name });
  editor.redo = [];
  if (!previous) updateTile(tile.id, (t) => (t.player = settings));
}

function undo(redo) {
  const stack = redo ? editor.redo : editor.undo;
  const step = stack.pop();
  if (!step) return;
  (redo ? editor.undo : editor.redo).push(step);
  updateTile(editor.id, (t) => (t.player = redo ? step.after : step.before));
}

function renderEditor() {
  const tile = editorTile();
  const root = $("editor");
  if (!tile) {
    root.replaceChildren(el("div", "empty", "Cette tuile n’existe plus"), button("Fermer", "btn", closeEditor));
    return;
  }
  if (!editor.built) {
    editor.built = true;
    buildEditor(root);
  }
  root.querySelector("h2").textContent = `Éditeur — ${tile.title}`;
  const play = root.querySelector("[data-play]");
  play.textContent = state.previewing ? "⏹ Arrêter" : "▶ Écouter";
  for (const b of root.querySelectorAll("[data-needs-selection]")) b.disabled = !editor.view.selection;
  redrawEditor();
}

function buildEditor(root) {
  const head = el("div", "editor-head");
  const v = () => editor.view.viewport;
  const t = () => editorTile();
  const sel = () => editor.view.selection;
  const logic = () => new TrimEditor(t().duration);
  const tool = (label, title, action, needsSelection) => {
    const b = button(label, "tool", () => {
      action();
      renderEditor();
    }, title);
    if (needsSelection) b.dataset.needsSelection = "1";
    return b;
  };
  const sep = () => el("span", "sep");
  const play = tool("▶ Écouter", `Écoute la sélection, ou le son découpé sans sélection (${MOD} + Entrée)`, () => togglePreview());
  play.dataset.play = "1";
  head.append(
    button("✕", "icon-btn", closeEditor, "Fermer (Échap)"),
    el("h2"),
    tool("−", `Zoom arrière (${MOD} + -)`, () => v().zoomOut()),
    tool("+", `Zoom avant (${MOD} + +)`, () => v().zoomIn()),
    tool("Ajuster", `Tout le son (${MOD} + Alt + -)`, () => v().zoomToFit()),
    tool("Zoom sélection", `${MOD} + Alt + =`, () => sel() && v().zoomTo(sel()), true),
    sep(),
    play,
    tool("⏪ 5 s", "Recule dans l’écoute ou la lecture en cours (←, Maj + ← pour 1 s)", () => call("scan", { id: editor.id, seconds: -SCAN_STEP })),
    tool("5 s ⏩", "Avance dans l’écoute ou la lecture en cours (→, Maj + → pour 1 s)", () => call("scan", { id: editor.id, seconds: SCAN_STEP })),
    sep(),
    tool("Recadrer", `${MOD} + Maj + X`, () => applyEdit(logic().cropping(sel(), t().player), "Recadrer"), true),
    tool("Fondu d’entrée", `${MOD} + Maj + I`, () => applyEdit(logic().fadingIn(sel(), t().player), "Fondu d’entrée"), true),
    tool("Fondu de sortie", `${MOD} + Maj + O`, () => applyEdit(logic().fadingOut(sel(), t().player), "Fondu de sortie"), true),
    sep(),
    tool("Revenir à l’original", "Retire découpe et fondus ; le fichier son n’est jamais modifié", () => applyEdit(logic().reverting(t().player), "Revenir à l’original")),
  );
  const main = el("div", "main-wave");
  const canvas = el("canvas");
  main.append(canvas);
  const overview = el("div", "overview");
  const overviewCanvas = el("canvas");
  overview.title = "Vue d’ensemble : cliquez ou glissez pour vous déplacer";
  overview.append(overviewCanvas);
  const foot = el("div", "editor-foot");
  foot.dataset.foot = "1";
  root.replaceChildren(head, main, overview, foot);

  handleDrags(canvas, t(), editor.view, redrawEditor, (draft, name, previous) => applyEdit(draft, name, previous), (down) => selectOrJump(canvas, down));
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    const box = canvas.getBoundingClientRect();
    if (e.ctrlKey || e.metaKey) v().zoomBy(Math.exp(-e.deltaY / 300), (e.clientX - box.left) / box.width);
    else v().scroll(((e.deltaX || e.deltaY) / box.width) * v().length);
    redrawEditor();
  }, { passive: false });
  const moveOverview = (e) => {
    const box = overviewCanvas.getBoundingClientRect();
    const centre = Math.min(Math.max((e.clientX - box.left) / box.width, 0), 1) * t().duration;
    v().scroll(centre - (v().visible.start + v().visible.end) / 2);
    redrawEditor();
  };
  overviewCanvas.addEventListener("pointerdown", (e) => {
    overviewCanvas.setPointerCapture(e.pointerId);
    moveOverview(e);
    const move = (m) => moveOverview(m);
    overviewCanvas.addEventListener("pointermove", move);
    overviewCanvas.addEventListener("pointerup", () => overviewCanvas.removeEventListener("pointermove", move), { once: true });
  });
  new ResizeObserver(() => redrawEditor()).observe(main);
}

/// Drag to select; a click without dragging clears the selection; Alt-click jumps the preview
/// or the playing tile there.
function selectOrJump(canvas, down) {
  const box = canvas.getBoundingClientRect();
  const vp = editor.view.viewport;
  const timeAt = (e) => vp.timeAt(e.clientX - box.left, box.width);
  if (down.altKey) {
    call("jump", { id: editor.id, time: timeAt(down) });
    return;
  }
  canvas.setPointerCapture(down.pointerId);
  const anchor = timeAt(down);
  editor.view.selection = null;
  const move = (e) => {
    const span = range(anchor, timeAt(e));
    editor.view.selection = span.end > span.start ? span : null;
    redrawEditor();
  };
  const up = () => {
    canvas.removeEventListener("pointermove", move);
    renderEditor();
  };
  canvas.addEventListener("pointermove", move);
  canvas.addEventListener("pointerup", up, { once: true });
  redrawEditor();
}

let playhead = null;

function redrawEditor() {
  if (!editor?.built) return;
  const tile = editorTile();
  if (!tile) return;
  const root = $("editor");
  const [main, overview] = root.querySelectorAll("canvas");
  if (!main) return;
  drawWave(main, { peaks: editor.peaks, tile, view: editor.view, playhead });
  const full = { viewport: new Viewport(tile.duration), draft: null, selection: null };
  drawWave(overview, { peaks: editor.peaks, tile, view: full, handles: false });
  const ctx = overview.getContext("2d");
  const ratio = window.devicePixelRatio || 1;
  const vis = editor.view.viewport.visible;
  ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue("--accent");
  ctx.lineWidth = 1.5 * ratio;
  ctx.strokeRect((vis.start / tile.duration) * overview.width, ratio, Math.max(((vis.end - vis.start) / tile.duration) * overview.width, 2), overview.height - 2 * ratio);
  const p = editor.view.draft ?? tile.player;
  const foot = root.querySelector("[data-foot]");
  const parts = [
    `Début : ${seconds(p.trimStart)}`,
    `Fin : ${seconds(tile.duration - p.trimEnd)}`,
    `Fondu d’entrée : ${seconds(p.fadeInTime)}`,
    `Fondu de sortie : ${seconds(p.fadeOutTime)}`,
  ];
  if (editor.view.selection) parts.push(`Sélection : ${seconds(editor.view.selection.end - editor.view.selection.start)}`);
  foot.replaceChildren(...parts.map((text) => el("span", null, text)), el("span", "hint", `Glissez pour sélectionner, Alt-clic pour déplacer la lecture, ${MOD} + molette pour zoomer ; les barres découpent, les points jaunes règlent les fondus.`));
}

function togglePreview() {
  const tile = editorTile();
  if (!tile) return;
  if (state.previewing) {
    call("stop_preview");
    return;
  }
  const selection = editor.view.selection;
  const start = selection ? selection.start : tile.player.trimStart;
  const end = selection ? selection.end : tile.duration - tile.player.trimEnd;
  call("preview", { id: tile.id, start, end });
}

// Follows the playhead while the editor is open.
setInterval(async () => {
  if (!editor) return;
  const tile = editorTile();
  const sounding = state.previewing || tile?.state === "playing";
  const next = sounding ? await invoke("playhead", { id: editor.id }).catch(() => null) : null;
  if (next !== playhead) {
    playhead = next;
    redrawEditor();
  }
}, 50);

// ---------- set options, rules, settings

function setOptions(anchor) {
  const set = state.set;
  const content = el("div");
  const modes = [
    ["stop", "Arrêter"],
    ["cueNext", "Sélectionner le son suivant"],
    ["continuous", "Lancer le son suivant"],
  ];
  content.append(el("strong", null, "À la fin d’un son"));
  for (const [value, label] of modes) {
    const row = el("label", "check");
    const radio = el("input");
    radio.type = "radio";
    radio.name = "playback-mode";
    radio.checked = set.playbackMode === value;
    radio.addEventListener("change", () => call("set_playback_mode", { mode: value }).then(() => setOptions(anchor)));
    row.append(radio, el("span", null, label));
    content.append(row);
  }
  const fade = el("label", "field");
  const slider = el("input");
  slider.type = "range";
  slider.min = 0;
  slider.max = 10;
  slider.step = 0.1;
  slider.value = set.crossfadeDuration;
  slider.disabled = set.playbackMode !== "continuous";
  const out = el("output", null, `${set.crossfadeDuration.toFixed(1).replace(".", ",")} s`);
  slider.addEventListener("input", () => {
    out.textContent = `${Number(slider.value).toFixed(1).replace(".", ",")} s`;
    call("set_crossfade", { seconds: Number(slider.value) });
  });
  fade.append(el("span", null, "Fondu enchaîné"), slider, out);
  content.append(fade, el("p", "help", "S’applique en mode liste. Le son suivant démarre ce temps avant la fin du son en cours (au plus la moitié de sa durée) et les deux se fondent l’un dans l’autre."));
  showPopover(anchor, content);
}

async function editRules(setId) {
  const summary = state.sets.find((s) => s.id === setId);
  if (!summary) return;
  if (state.set.id !== setId) await call("select_set", { id: setId });
  const rules = structuredClone(state.set.smartRules ?? { match: "all", rules: [] });
  const body = el("div");
  const count = el("p", "help");
  const list = el("div");
  const refreshCount = async () => {
    const n = await invoke("count_matching", { rules }).catch(() => 0);
    count.textContent = `${n} tuile${n > 1 ? "s" : ""} des sets normaux correspond${n > 1 ? "ent" : ""} à ces règles.`;
  };
  const select = (options, value, onChange) => {
    const s = el("select");
    for (const [v, label] of options) {
      const o = el("option", null, label);
      o.value = v;
      s.append(o);
    }
    s.value = value;
    s.addEventListener("change", () => onChange(s.value));
    return s;
  };
  const draw = () => {
    list.replaceChildren();
    rules.rules.forEach((rule, i) => {
      const row = el("div", "rule");
      const kind = select([["title", "Titre"], ["color", "Couleur"], ["flag", "Indicateur"]], rule.kind, (k) => {
        rules.rules[i] = k === "title" ? { kind: k, comparison: "contains", text: "" } : k === "color" ? { kind: k, color: "purple" } : { kind: k, flag: "favorite", value: true };
        draw();
      });
      row.append(kind);
      if (rule.kind === "title") {
        row.append(select([["contains", "contient"], ["beginsWith", "commence par"], ["endsWith", "finit par"]], rule.comparison, (c) => ((rule.comparison = c), refreshCount())));
        const text = el("input");
        text.value = rule.text;
        text.addEventListener("input", () => ((rule.text = text.value), refreshCount()));
        text.addEventListener("keydown", (e) => e.stopPropagation());
        row.append(text);
      } else if (rule.kind === "color") {
        row.append(select(COLORS.map((c) => [c, COLOR_NAMES[c]]), rule.color, (c) => ((rule.color = c), refreshCount())));
      } else {
        row.append(
          select([["true", "est"], ["false", "n’est pas"]], String(rule.value), (v) => ((rule.value = v === "true"), refreshCount())),
          select([["favorite", "favori"], ["loops", "en boucle"], ["holdToPlay", "maintenir pour jouer"], ["solo", "solo"], ["played", "déjà joué"]], rule.flag, (f) => ((rule.flag = f), refreshCount())),
        );
      }
      row.append(button("✕", "icon-btn", () => (rules.rules.splice(i, 1), draw()), "Retirer la règle"));
      list.append(row);
    });
    refreshCount();
  };
  const matchRow = el("div", "rule");
  matchRow.append(el("span", null, "Montrer les tuiles qui suivent"), select([["all", "toutes les règles"], ["any", "au moins une règle"]], rules.match, (m) => ((rules.match = m), refreshCount())));
  body.append(el("h2", null, `Règles de « ${summary.title} »`), matchRow, list, button("+ Ajouter une règle", "btn", () => (rules.rules.push({ kind: "flag", flag: "favorite", value: true }), draw())), count);
  draw();
  const ok = await openDialog(body, [
    { label: "Annuler", value: false },
    { label: "OK", primary: true, value: true },
  ]);
  if (ok) call("set_rules", { id: setId, rules });
}

async function openSettings(tab = "general") {
  const settings = await call("settings");
  if (!settings) return;
  const body = el("div");
  const tabs = el("div", "tabs");
  const panes = {};
  for (const [id, label] of [["general", "Général"], ["midi", "MIDI"], ["osc", "OSC"]]) {
    const b = button(label, "btn", () => show(id));
    b.dataset.tab = id;
    tabs.append(b);
    panes[id] = el("div");
  }
  const show = (id) => {
    for (const b of tabs.children) b.classList.toggle("on", b.dataset.tab === id);
    for (const [key, pane] of Object.entries(panes)) pane.hidden = key !== id;
  };
  body.append(el("h2", null, "Réglages"), tabs, ...Object.values(panes));

  // General: global shortcut and links.
  const g = panes.general;
  const shortcutRow = el("div", "field");
  const record = button(settings.globalShortcut ?? "Aucun — cliquer pour enregistrer", "btn", () => {
    record.textContent = "Tapez le raccourci…";
    const onKey = async (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") return stop(settings.globalShortcut ?? "Aucun — cliquer pour enregistrer");
      if (e.key === "Backspace" || e.key === "Delete") {
        await call("set_global_shortcut", { accelerator: null });
        settings.globalShortcut = null;
        return stop("Aucun — cliquer pour enregistrer");
      }
      const result = accelerator(e);
      if (!result) return;
      if (result.error) {
        problem.textContent = result.error;
        return;
      }
      try {
        await invoke("set_global_shortcut", { accelerator: result.accelerator });
        settings.globalShortcut = result.accelerator;
        problem.textContent = "";
        stop(result.accelerator);
      } catch (error) {
        problem.textContent = String(error);
      }
    };
    const stop = (label) => {
      window.removeEventListener("keydown", onKey, true);
      record.textContent = label;
    };
    window.addEventListener("keydown", onKey, true);
  });
  const problem = el("p", "help error-text");
  shortcutRow.append(el("span", null, "Ramener au premier plan"), record);
  const links = el("div", "links");
  for (const [link, meaning] of [
    ["soundboard://play?title=Nom", "Lance la tuile (sans l’arrêter si elle joue)"],
    ["soundboard://stop?title=Nom", "Arrête la tuile"],
    ["soundboard://fade?title=Nom", "Arrête la tuile en fondu"],
    ["…&set=Nom du set", "Cherche la tuile dans ce set seulement"],
    ["soundboard://set?title=Nom", "Affiche le set"],
    ["soundboard://transport/stopAll", "Tout arrêter"],
    ["soundboard://master/volume?value=0.5", "Règle le volume général (0 à 1)"],
  ]) {
    links.append(el("code", null, link), el("span", null, meaning));
  }
  g.append(
    el("strong", null, "Raccourci global"),
    shortcutRow,
    problem,
    el("p", "help", "Fonctionne même quand une autre app est au premier plan. Il faut Ctrl, Alt ou la touche Windows/Commande. Pendant l’enregistrement : Échap annule, Retour arrière efface le raccourci."),
    el("strong", null, "Liens soundboard://"),
    links,
    el("p", "help", "À lancer depuis un Stream Deck, un script ou un raccourci. Les noms ignorent majuscules et accents. Toutes les adresses OSC marchent aussi comme liens."),
  );

  // MIDI: pad lights.
  const m = panes.midi;
  const device = el("select");
  for (const [value, label] of [["", "Aucun"], ...settings.midiOutputs.map((n) => [n, n])]) {
    const o = el("option", null, label);
    o.value = value;
    device.append(o);
  }
  if (settings.midiOutput && !settings.midiOutputs.includes(settings.midiOutput)) {
    const o = el("option", null, `${settings.midiOutput} (débranché)`);
    o.value = settings.midiOutput;
    device.append(o);
  }
  device.value = settings.midiOutput ?? "";
  const velocity = el("input");
  velocity.type = "range";
  velocity.min = 1;
  velocity.max = 127;
  velocity.value = settings.midiVelocity;
  const velocityOut = el("output", null, String(settings.midiVelocity));
  const midiProblem = el("p", "help error-text");
  const applyMidi = async () => {
    velocityOut.textContent = velocity.value;
    velocity.disabled = !device.value;
    try {
      await invoke("set_midi_output", { name: device.value || null, velocity: Number(velocity.value) });
      midiProblem.textContent = "";
    } catch (error) {
      midiProblem.textContent = String(error);
    }
  };
  velocity.disabled = !device.value;
  device.addEventListener("change", applyMidi);
  velocity.addEventListener("change", applyMidi);
  velocity.addEventListener("input", () => (velocityOut.textContent = velocity.value));
  const deviceRow = el("label", "field");
  deviceRow.append(el("span", null, "Appareil"), device);
  const velocityRow = el("label", "field");
  velocityRow.append(el("span", null, "Vélocité « allumé »"), velocity, velocityOut);
  m.append(
    el("strong", null, "Allumer les pads du contrôleur"),
    deviceRow,
    velocityRow,
    midiProblem,
    el("p", "help", "Quand une tuile associée à une note joue, l’app envoie cette note au contrôleur pour allumer le pad, et l’éteint à l’arrêt. Sur beaucoup de contrôleurs à pads, la vélocité choisit la couleur."),
    el("p", "help", `Entrées : ${settings.midiSources} source${settings.midiSources > 1 ? "s" : ""} MIDI écoutée${settings.midiSources > 1 ? "s" : ""}. Les contrôleurs branchés en cours de route sont pris en compte après quelques secondes.`),
  );

  // OSC: reception, feedback and addresses.
  const o = panes.osc;
  const osc = structuredClone(settings.osc);
  const status = el("span", null, settings.oscStatus);
  const applyOsc = async () => {
    status.textContent = (await call("set_osc", { osc })) ?? status.textContent;
  };
  const toggleRow = (label, key) => {
    const row = el("label", "check");
    const box = el("input");
    box.type = "checkbox";
    box.checked = osc[key];
    box.addEventListener("change", () => ((osc[key] = box.checked), applyOsc()));
    row.append(box, el("span", null, label));
    return row;
  };
  const inputRow = (label, key, number) => {
    const row = el("label", "field");
    const input = el("input");
    input.value = osc[key];
    if (number) input.inputMode = "numeric";
    input.addEventListener("keydown", (e) => e.stopPropagation());
    input.addEventListener("change", () => {
      osc[key] = number ? Math.max(0, Math.min(65535, Number.parseInt(input.value, 10) || 0)) : input.value.trim();
      input.value = osc[key];
      applyOsc();
    });
    row.append(el("span", null, label), input);
    return row;
  };
  const statusRow = el("div", "field");
  statusRow.append(el("span", null, "État"), status);
  const addresses = el("div", "links");
  for (const [address, meaning] of [
    ["/set/<n>/tile/<colonne>/<ligne>/play", "Lance ou arrête la tuile (0 relâche une tuile « maintenir pour jouer »)"],
    ["…/fade, …/loop, …/mute, …/solo, …/toggleAB", "Fondu de sortie, boucle, muet, solo, volume A/B"],
    ["/set/selected/tile/…", "Le set affiché"],
    ["/master/volume 0…1", "Volume général"],
    ["/master/toggleAB, /master/volumeUp, /master/volumeDown", "Préréglage et pas du volume général"],
    ["/transport/playPauseAll, /transport/stopAll", "Pause générale, tout arrêter"],
    ["/global/bringForward", "Ramène l’app au premier plan"],
  ]) {
    addresses.append(el("code", null, address), el("span", null, meaning));
  }
  o.append(
    el("strong", null, "Réception"),
    toggleRow("Recevoir l’OSC", "inputEnabled"),
    inputRow("Port", "inputPort", true),
    statusRow,
    el("p", "help", "Pilotez l’app depuis un téléphone, une tablette (TouchOSC…) ou un autre logiciel sur le même réseau. Le pare-feu peut demander l’autorisation à l’activation."),
    el("strong", null, "Envoi"),
    toggleRow("Envoyer l’état des tuiles", "outputEnabled"),
    inputRow("Adresse", "outputHost", false),
    inputRow("Port", "outputPort", true),
    el("p", "help", "Envoie …/play 1 quand une tuile démarre et 0 quand elle s’arrête."),
    el("strong", null, "Adresses"),
    addresses,
  );

  show(tab);
  await openDialog(body, [{ label: "Fermer", primary: true, value: true }]);
}

function appMenu(anchor) {
  const box = anchor.getBoundingClientRect();
  const smart = !!state.set.smartRules;
  showMenu(box.left, box.bottom + 4, [
    { label: "Nouveau set", shortcut: `${MOD} + Maj + N`, action: () => call("add_set") },
    { label: "Nouveau smart set", shortcut: `${MOD} + Alt + N`, action: newSmartSet },
    { label: "Modifier les règles…", disabled: !smart, action: () => editRules(state.set.id) },
    "sep",
    { label: "Ajouter des sons…", disabled: smart, action: () => call("choose_sounds") },
    { label: "Importer des sets…", action: () => call("import_sets") },
    { label: "Exporter le set…", action: () => call("export_set", { title: state.set.title }) },
    "sep",
    { label: "Jouer la sélection", shortcut: `${MOD} + Entrée`, action: () => call("transport", { action: "playSelection" }) },
    { label: "Réinitialiser la liste", shortcut: `${MOD} + R`, action: () => call("reset_list") },
    { label: `Fondu vers le volume ${state.master.usesVolumeB ? "A" : "B"}`, shortcut: `${MOD} + Maj + D`, action: () => call("master_action", { action: "toggleAB" }) },
    "sep",
    { label: "Set précédent", shortcut: `${MOD} + Maj + [`, action: () => call("step_set", { forward: false }) },
    { label: "Set suivant", shortcut: `${MOD} + Maj + ]`, action: () => call("step_set", { forward: true }) },
    "sep",
    { label: "Réglages…", shortcut: `${MOD} + ,`, action: () => openSettings() },
  ]);
}

async function newSmartSet() {
  const id = await call("add_smart_set");
  if (id) editRules(id);
}

// ---------- keyboard

function onKeyDown(e) {
  if ($("dialog").open) return;
  if (e.key === "Escape" && (!$("menu").hidden || !$("popover").hidden)) return closeMenus();
  const mod = MAC ? e.metaKey : e.ctrlKey;
  if (editor && editorKey(e, mod)) return e.preventDefault();
  if (isTyping(e.target)) return;
  const shortcuts = [
    [mod && e.key === "Enter", () => call("transport", { action: "playSelection" })],
    [mod && !e.shiftKey && e.code === "KeyR", () => call("reset_list")],
    [mod && e.shiftKey && e.code === "KeyD", () => call("master_action", { action: "toggleAB" })],
    [mod && e.key === "ArrowUp", () => call("master_action", { action: "up" })],
    [mod && e.key === "ArrowDown", () => call("master_action", { action: "down" })],
    [mod && e.code === "KeyL", () => state.selected && call("midi_learn", { id: state.selected.id })],
    [mod && e.shiftKey && e.code === "KeyN", () => call("add_set")],
    [mod && e.altKey && e.code === "KeyN", newSmartSet],
    [mod && e.shiftKey && e.code === "BracketLeft", () => call("step_set", { forward: false })],
    [mod && e.shiftKey && e.code === "BracketRight", () => call("step_set", { forward: true })],
    [mod && e.code === "KeyI", () => ((inspectorOpen = !inspectorOpen), render())],
    [mod && e.shiftKey && e.code === "KeyE", () => state.selected && openEditor(state.selected.id)],
    [mod && e.key === ",", () => openSettings()],
    [!mod && !e.altKey && e.key === " ", () => call("transport", { action: "pauseAll" })],
    [!mod && !e.altKey && e.key === "-", () => call("transport", { action: "fadeOutAll" })],
    [!mod && !e.altKey && e.key === "Escape", () => call("transport", { action: "stopAll" })],
  ];
  const hit = shortcuts.find(([when]) => when);
  if (hit) {
    e.preventDefault();
    if (!e.repeat) hit[1]();
    return;
  }
  // Tile keys: one letter or digit, without Ctrl/⌘/Alt; Shift pauses.
  if (mod || e.altKey || e.ctrlKey || e.metaKey || e.repeat || e.key.length !== 1) return;
  e.preventDefault();
  invoke("hotkey", { key: tileKey(e), down: true, shift: e.shiftKey });
}

function onKeyUp(e) {
  if ($("dialog").open || isTyping(e.target) || e.ctrlKey || e.metaKey || e.altKey || e.key.length !== 1) return;
  invoke("hotkey", { key: tileKey(e), down: false, shift: e.shiftKey });
}

/// The top-row digits by position, so they work on layouts where they need Shift (AZERTY).
function tileKey(e) {
  return /^Digit\d$/.test(e.code) ? e.code.slice(5) : e.key;
}

/// Keys of the open editor; true when handled.
function editorKey(e, mod) {
  const v = editor.view.viewport;
  const tile = editorTile();
  if (!tile) return false;
  const logic = new TrimEditor(tile.duration);
  const sel = editor.view.selection;
  const actions = [
    [e.key === "Escape", closeEditor],
    [mod && e.key === "Enter", togglePreview],
    [mod && e.altKey && e.key === "-", () => v.zoomToFit()],
    [mod && e.altKey && (e.key === "=" || e.key === "+"), () => sel && v.zoomTo(sel)],
    [mod && !e.altKey && e.key === "-", () => v.zoomOut()],
    [mod && !e.altKey && (e.key === "=" || e.key === "+"), () => v.zoomIn()],
    [mod && e.shiftKey && e.code === "KeyX", () => sel && applyEdit(logic.cropping(sel, tile.player), "Recadrer")],
    [mod && e.shiftKey && e.code === "KeyI", () => sel && applyEdit(logic.fadingIn(sel, tile.player), "Fondu d’entrée")],
    [mod && e.shiftKey && e.code === "KeyO", () => sel && applyEdit(logic.fadingOut(sel, tile.player), "Fondu de sortie")],
    [mod && e.shiftKey && e.code === "KeyZ", () => undo(true)],
    [mod && !e.shiftKey && e.code === "KeyZ", () => undo(false)],
    [!mod && e.key === "ArrowLeft", () => call("scan", { id: editor.id, seconds: -(e.shiftKey ? FINE_SCAN_STEP : SCAN_STEP) })],
    [!mod && e.key === "ArrowRight", () => call("scan", { id: editor.id, seconds: e.shiftKey ? FINE_SCAN_STEP : SCAN_STEP })],
  ];
  const hit = actions.find(([when]) => when);
  if (!hit) return isTyping(e.target) ? false : e.key.length === 1 && !mod;
  hit[1]();
  if (editor) renderEditor();
  return true;
}

// ---------- wiring

$("add-set").addEventListener("click", () => call("add_set"));
$("app-menu").addEventListener("click", (e) => appMenu(e.currentTarget));
$("mode-grid").addEventListener("click", () => call("set_display_mode", { mode: "grid" }));
$("mode-list").addEventListener("click", () => call("set_display_mode", { mode: "list" }));
$("set-options").addEventListener("click", (e) => setOptions(e.currentTarget));
$("add-sounds").addEventListener("click", () => call("choose_sounds"));
$("master-a").addEventListener("click", () => call("set_master", { master: { ...state.master, usesVolumeB: false } }));
$("master-b").addEventListener("click", () => call("set_master", { master: { ...state.master, usesVolumeB: true } }));
$("master-volume").addEventListener("input", (e) => {
  const value = Number(e.target.value);
  const master = { ...state.master, [state.master.usesVolumeB ? "volumeB" : "volumeA"]: value };
  call("set_master", { master });
});
$("master-mute").addEventListener("click", () => call("master_action", { action: "mute" }));
$("fade-all").addEventListener("click", () => call("transport", { action: "fadeOutAll" }));
$("pause-all").addEventListener("click", () => call("transport", { action: "pauseAll" }));
$("stop-all").addEventListener("click", () => call("transport", { action: "stopAll" }));
$("toggle-inspector").addEventListener("click", () => {
  inspectorOpen = !inspectorOpen;
  render();
});
$("board").addEventListener("click", (e) => {
  if (e.target === e.currentTarget) call("select_tile", { id: null });
});
window.addEventListener("keydown", onKeyDown);
window.addEventListener("keyup", onKeyUp);
window.addEventListener("pointerdown", (e) => {
  if (!e.target.closest("#menu, #popover")) closeMenus();
});
window.addEventListener("blur", closeMenus);
window.addEventListener("resize", () => editor && redrawEditor());
document.addEventListener("contextmenu", (e) => {
  if (!isTyping(e.target)) e.preventDefault();
});

listen("board", (event) => {
  state = event.payload;
  render();
});
listen("tauri://drag-enter", () => ($("drop-hint").hidden = false));
listen("tauri://drag-leave", () => ($("drop-hint").hidden = true));
listen("tauri://drag-drop", () => ($("drop-hint").hidden = true));

invoke("snapshot").then((snapshot) => {
  state = snapshot;
  render();
});

// A held tile whose pointer was lost must not freeze redraws.
window.addEventListener("pointerup", () => {
  if (renderPending && !holding) {
    renderPending = false;
    render();
  }
});
