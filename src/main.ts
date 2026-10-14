import { invoke } from "@tauri-apps/api/core";

type Engine = "cloud" | "local";

type Settings = {
  hotkey: string;
  input_device: string | null;
  cleanup_model: string;
  engine: Engine;
  local_model: string;
};

type LocalModel = { id: string; label: string; size_mb: number };

const hotkeyButton = document.querySelector<HTMLButtonElement>("#hotkey")!;
const hotkeyMessage = document.querySelector<HTMLElement>("#hotkey-message")!;
const microphone = document.querySelector<HTMLSelectElement>("#microphone")!;
const modelInput = document.querySelector<HTMLInputElement>("#model")!;
const hotkeyReset = document.querySelector<HTMLButtonElement>("#hotkey-reset")!;
const microphoneReset = document.querySelector<HTMLButtonElement>("#microphone-reset")!;
const modelReset = document.querySelector<HTMLButtonElement>("#model-reset")!;
const engineCloud = document.querySelector<HTMLButtonElement>("#engine-cloud")!;
const engineLocal = document.querySelector<HTMLButtonElement>("#engine-local")!;
const localModelSelect = document.querySelector<HTMLSelectElement>("#local-model")!;
const engineHint = document.querySelector<HTMLElement>("#engine-hint")!;
const transcriptionReset = document.querySelector<HTMLButtonElement>("#transcription-reset")!;
const savedNote = document.querySelector<HTMLElement>("#saved")!;

const SYSTEM_DEFAULT = "";
let settings: Settings;
let defaults: Settings;
let recording = false;
let savedTimer: number | undefined;

// ---- helpers -------------------------------------------------------------

function flashSaved(): void {
  savedNote.textContent = "Saved";
  savedNote.style.opacity = "1";
  window.clearTimeout(savedTimer);
  savedTimer = window.setTimeout(() => {
    savedNote.style.opacity = "0";
  }, 1500);
}

function setHotkeyMessage(text: string, isError = false): void {
  hotkeyMessage.textContent = text;
  hotkeyMessage.classList.toggle("error", isError);
}

const MODIFIER_SYMBOLS: Record<string, string> = {
  Control: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Command: "⌘",
};

/** "Alt+Space" -> "⌥ Space" */
function prettyHotkey(hotkey: string): string {
  return hotkey
    .split("+")
    .map((part) => MODIFIER_SYMBOLS[part] ?? part.replace(/^Key/, "").replace(/^Digit/, ""))
    .join(" ");
}

function renderHotkey(): void {
  hotkeyButton.textContent = prettyHotkey(settings.hotkey);
  updateResetButtons();
}

/** A reset button is only useful while its setting differs from the default. */
function updateResetButtons(): void {
  hotkeyReset.disabled = settings.hotkey === defaults.hotkey;
  microphoneReset.disabled = settings.input_device === defaults.input_device;
  modelReset.disabled = settings.cleanup_model === defaults.cleanup_model;
  transcriptionReset.disabled =
    settings.engine === defaults.engine && settings.local_model === defaults.local_model;
}

// ---- hotkey recording ----------------------------------------------------

const MODIFIER_CODES = new Set([
  "AltLeft",
  "AltRight",
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "MetaLeft",
  "MetaRight",
]);

function modifiersOf(e: KeyboardEvent): string[] {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Command");
  return mods;
}

async function startRecording(): Promise<void> {
  if (recording) return;
  recording = true;
  hotkeyButton.classList.add("recording");
  hotkeyButton.textContent = "Press a shortcut…";
  setHotkeyMessage("Esc to cancel. Include ⌥, ⌃ or ⌘ (or use an F-key).");
  // Release the current hotkey so pressing it doesn't start a dictation.
  await invoke("pause_hotkey");
}

async function stopRecording(): Promise<void> {
  if (!recording) return;
  recording = false;
  hotkeyButton.classList.remove("recording");
  renderHotkey();
  await invoke("resume_hotkey");
}

async function onKeyDown(e: KeyboardEvent): Promise<void> {
  if (!recording) return;
  e.preventDefault();
  e.stopPropagation();

  if (e.code === "Escape") {
    setHotkeyMessage("");
    await stopRecording();
    return;
  }
  if (MODIFIER_CODES.has(e.code)) {
    // Show the modifiers being held while waiting for the main key.
    hotkeyButton.textContent = modifiersOf(e).map((m) => MODIFIER_SYMBOLS[m]).join(" ") + " …";
    return;
  }

  const mods = modifiersOf(e);
  const isFunctionKey = /^F\d{1,2}$/.test(e.code);
  const hasRealModifier = mods.some((m) => m !== "Shift");
  if (!hasRealModifier && !isFunctionKey) {
    // Without this, e.g. plain "A" would be swallowed system-wide.
    setHotkeyMessage("Add ⌥, ⌃ or ⌘ so normal typing isn't blocked.", true);
    return;
  }

  const candidate = [...mods, e.code].join("+");
  await stopRecording();
  await applyHotkey(candidate);
}

async function applyHotkey(hotkey: string): Promise<void> {
  try {
    settings = await invoke<Settings>("set_hotkey", { hotkey });
    renderHotkey();
    setHotkeyMessage("");
    flashSaved();
  } catch (err) {
    setHotkeyMessage(String(err), true);
  }
}

hotkeyButton.addEventListener("click", () => void startRecording());
hotkeyReset.addEventListener("click", () => void applyHotkey(defaults.hotkey));
hotkeyButton.addEventListener("blur", () => void stopRecording());
window.addEventListener("keydown", (e) => void onKeyDown(e), true);

// ---- microphone ----------------------------------------------------------

async function loadDevices(): Promise<void> {
  const devices = await invoke<string[]>("list_input_devices");
  const current = settings.input_device;

  microphone.replaceChildren();
  const addOption = (value: string, label: string): void => {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    microphone.append(option);
  };

  addOption(SYSTEM_DEFAULT, "System default");
  for (const name of devices) addOption(name, name);
  if (current && !devices.includes(current)) {
    addOption(current, `${current} (not connected)`);
  }
  microphone.value = current ?? SYSTEM_DEFAULT;
  updateResetButtons();
}

async function saveDevice(device: string | null): Promise<void> {
  settings = await invoke<Settings>("set_input_device", { device });
  await loadDevices();
  flashSaved();
}

microphone.addEventListener("change", () => {
  void saveDevice(microphone.value === SYSTEM_DEFAULT ? null : microphone.value);
});
microphoneReset.addEventListener("click", () => void saveDevice(defaults.input_device));

// ---- clean-up model ------------------------------------------------------

async function saveModel(value: string): Promise<void> {
  settings = await invoke<Settings>("set_cleanup_model", { model: value });
  modelInput.value = settings.cleanup_model;
  updateResetButtons();
  flashSaved();
}

modelInput.addEventListener("change", () => void saveModel(modelInput.value));
modelInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") modelInput.blur();
});
modelReset.addEventListener("click", () => void saveModel(defaults.cleanup_model));

// ---- transcription engine ------------------------------------------------

const ENGINE_HINTS: Record<Engine, string> = {
  cloud: "OpenRouter's hosted Whisper. Audio is sent online; about $0.006 per minute.",
  local: "Runs on this Mac. Free, and your audio never leaves it.",
};

async function loadLocalModels(): Promise<void> {
  const models = await invoke<LocalModel[]>("list_local_models");
  localModelSelect.replaceChildren();
  for (const m of models) {
    const option = document.createElement("option");
    option.value = m.id;
    option.textContent = `${m.label} · ${m.size_mb} MB`;
    localModelSelect.append(option);
  }
}

function renderEngine(): void {
  const local = settings.engine === "local";
  engineCloud.setAttribute("aria-checked", String(!local));
  engineLocal.setAttribute("aria-checked", String(local));
  localModelSelect.hidden = !local;
  localModelSelect.value = settings.local_model;
  engineHint.textContent = ENGINE_HINTS[settings.engine];
  updateResetButtons();
}

async function chooseEngine(engine: Engine): Promise<void> {
  if (settings.engine === engine) return;
  settings = await invoke<Settings>("set_engine", { engine });
  renderEngine();
  flashSaved();
}

engineCloud.addEventListener("click", () => void chooseEngine("cloud"));
engineLocal.addEventListener("click", () => void chooseEngine("local"));

localModelSelect.addEventListener("change", async () => {
  settings = await invoke<Settings>("set_local_model", { model: localModelSelect.value });
  renderEngine();
  flashSaved();
});

transcriptionReset.addEventListener("click", async () => {
  await invoke<Settings>("set_engine", { engine: defaults.engine });
  settings = await invoke<Settings>("set_local_model", { model: defaults.local_model });
  renderEngine();
  flashSaved();
});

// ---- init ----------------------------------------------------------------

async function init(): Promise<void> {
  defaults = await invoke<Settings>("get_defaults");
  settings = await invoke<Settings>("get_settings");
  renderHotkey();
  modelInput.value = settings.cleanup_model;
  await loadLocalModels();
  renderEngine();
  await loadDevices();
}

// Microphones come and go; refresh the list whenever the window is focused.
window.addEventListener("focus", () => void loadDevices());
void init();
