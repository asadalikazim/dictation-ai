import { invoke } from "@tauri-apps/api/core";

type Engine = "cloud" | "local";

type Settings = {
  hotkey: string;
  input_device: string | null;
  cleanup_model: string;
  cleanup_enabled: boolean;
  engine: Engine;
  cloud_transcription_model: string;
  local_model: string;
};

type LocalModel = { id: string; label: string; size_mb: number };
type ModelOption = { id: string; label: string };

type KeyStatus = { set: boolean; hint: string | null };
type KeyCheck =
  | { status: "no_key" }
  | { status: "valid"; usage: number | null; limit_remaining: number | null }
  | { status: "rejected" }
  | { status: "unreachable"; message: string };
type CloudCatalog = { transcription: ModelOption[]; cleanup: ModelOption[] };

const hotkeyButton = document.querySelector<HTMLButtonElement>("#hotkey")!;
const hotkeyMessage = document.querySelector<HTMLElement>("#hotkey-message")!;
const microphone = document.querySelector<HTMLSelectElement>("#microphone")!;
const modelInput = document.querySelector<HTMLInputElement>("#model")!;
const cleanupSelect = document.querySelector<HTMLSelectElement>("#cleanup-model")!;
const cleanupOn = document.querySelector<HTMLButtonElement>("#cleanup-on")!;
const cleanupOff = document.querySelector<HTMLButtonElement>("#cleanup-off")!;
const cleanupHint = document.querySelector<HTMLElement>("#cleanup-hint")!;
const cloudSelect = document.querySelector<HTMLSelectElement>("#cloud-model")!;
const cloudOther = document.querySelector<HTMLInputElement>("#cloud-model-other")!;
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

// ---- OpenRouter key ------------------------------------------------------

const keyEntry = document.querySelector<HTMLElement>("#key-entry")!;
const keyInput = document.querySelector<HTMLInputElement>("#api-key")!;
const keySave = document.querySelector<HTMLButtonElement>("#key-save")!;
const keySaved = document.querySelector<HTMLElement>("#key-saved")!;
const keyHint = document.querySelector<HTMLElement>("#key-hint")!;
const keyActions = document.querySelector<HTMLElement>("#key-actions")!;
const keyCheck = document.querySelector<HTMLButtonElement>("#key-check")!;
const keyReplace = document.querySelector<HTMLButtonElement>("#key-replace")!;
const keyRemove = document.querySelector<HTMLButtonElement>("#key-remove")!;
const keyMessage = document.querySelector<HTMLElement>("#key-message")!;

function setKeyMessage(text: string, tone: "" | "ok" | "error" = ""): void {
  keyMessage.textContent = text;
  keyMessage.classList.toggle("ok", tone === "ok");
  keyMessage.classList.toggle("error", tone === "error");
}

/** Show the entry field, or the saved summary with its actions. */
function renderKey(status: KeyStatus, entering = false): void {
  const showEntry = !status.set || entering;
  keyEntry.hidden = !showEntry;
  keySaved.hidden = showEntry;
  keyActions.hidden = showEntry;
  keyHint.textContent = status.hint ?? "";
  keyInput.value = "";
  if (showEntry) keyInput.focus();
}

function formatUsd(amount: number): string {
  return `$${amount.toFixed(amount < 1 ? 4 : 2)}`;
}

async function saveKey(): Promise<void> {
  const key = keyInput.value;
  keySave.disabled = true;
  try {
    const status = await invoke<KeyStatus>("set_api_key", { key });
    renderKey(status);
    setKeyMessage("");
    flashSaved();
  } catch (err) {
    setKeyMessage(String(err), "error");
  } finally {
    keySave.disabled = false;
  }
}

async function checkKey(): Promise<void> {
  setKeyMessage("Checking…");
  keyCheck.disabled = true;
  try {
    const result = await invoke<KeyCheck>("check_api_key");
    switch (result.status) {
      case "valid": {
        const used = result.usage === null ? "" : ` Used ${formatUsd(result.usage)}.`;
        const left =
          result.limit_remaining === null
            ? " No spending limit on this key."
            : ` ${formatUsd(result.limit_remaining)} left under this key's limit.`;
        setKeyMessage(`Valid.${used}${left}`, "ok");
        break;
      }
      case "rejected":
        setKeyMessage("OpenRouter rejected this key. Replace it with a valid one.", "error");
        break;
      case "no_key":
        setKeyMessage("No key is saved yet.", "error");
        break;
      case "unreachable":
        setKeyMessage(`Could not check the key: ${result.message}`, "error");
        break;
    }
  } finally {
    keyCheck.disabled = false;
  }
}

async function removeKey(): Promise<void> {
  try {
    const status = await invoke<KeyStatus>("clear_api_key");
    renderKey(status);
    setKeyMessage("Key removed. Add one to use clean-up and cloud transcription.");
  } catch (err) {
    setKeyMessage(String(err), "error");
  }
}

keySave.addEventListener("click", () => void saveKey());
keyInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") void saveKey();
});
keyCheck.addEventListener("click", () => void checkKey());
keyReplace.addEventListener("click", () => {
  setKeyMessage("");
  renderKey({ set: true, hint: keyHint.textContent }, true);
});
keyRemove.addEventListener("click", () => void removeKey());

async function loadKey(): Promise<void> {
  const status = await invoke<KeyStatus>("get_api_key_status");
  renderKey(status);
  if (!status.set) {
    setKeyMessage("Add your key to enable clean-up and cloud transcription.");
  }
}

// ---- helpers -------------------------------------------------------------

function flashSaved(text = "Saved", isError = false): void {
  savedNote.textContent = text;
  savedNote.classList.toggle("error", isError);
  savedNote.style.opacity = "1";
  window.clearTimeout(savedTimer);
  savedTimer = window.setTimeout(() => {
    savedNote.style.opacity = "0";
  }, isError ? 4000 : 1500);
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
  modelReset.disabled =
    settings.cleanup_model === defaults.cleanup_model &&
    settings.cleanup_enabled === defaults.cleanup_enabled;
  transcriptionReset.disabled =
    settings.engine === defaults.engine &&
    settings.local_model === defaults.local_model &&
    settings.cloud_transcription_model === defaults.cloud_transcription_model;
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

// ---- model pickers -------------------------------------------------------

const OTHER = "__other__";

type Picker = { render: () => void };

/**
 * A drop-down of known model ids with an "Other…" entry that reveals a text
 * field for any OpenRouter model id. A saved id that is not in the list is
 * shown under "Other…" with its text filled in.
 */
function createModelPicker(opts: {
  select: HTMLSelectElement;
  input: HTMLInputElement;
  options: ModelOption[];
  current: () => string;
  save: (id: string) => Promise<void>;
}): Picker {
  const { select, input, options, current, save } = opts;
  const isKnown = (id: string): boolean => options.some((o) => o.id === id);

  select.replaceChildren();
  for (const o of [...options, { id: OTHER, label: "Other…" }]) {
    const option = document.createElement("option");
    option.value = o.id;
    option.textContent = o.label;
    select.append(option);
  }

  function render(): void {
    const id = current();
    const known = isKnown(id);
    select.value = known ? id : OTHER;
    input.hidden = known;
    if (!known) input.value = id;
  }

  select.addEventListener("change", () => {
    if (select.value === OTHER) {
      input.hidden = false;
      input.value = isKnown(current()) ? "" : current();
      input.focus();
      return;
    }
    void save(select.value);
  });
  input.addEventListener("change", () => {
    const value = input.value.trim();
    if (value) void save(value);
    else render();
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") input.blur();
  });

  return { render };
}

let cleanupPicker: Picker;
let cloudPicker: Picker;

async function saveCleanupModel(model: string): Promise<void> {
  try {
    settings = await invoke<Settings>("set_cleanup_model", { model });
    flashSaved();
  } catch (err) {
    flashSaved(String(err), true);
  }
  cleanupPicker.render();
  updateResetButtons();
}

async function saveCloudModel(model: string): Promise<void> {
  try {
    settings = await invoke<Settings>("set_cloud_transcription_model", { model });
    flashSaved();
  } catch (err) {
    flashSaved(String(err), true);
  }
  cloudPicker.render();
  updateResetButtons();
}

const CLEANUP_HINTS = {
  on: "Tidies the transcript: removes fillers, fixes punctuation.",
  off: "Off: the raw transcript is pasted exactly as transcribed.",
};

/** Show the model controls only while clean-up is on. */
function renderCleanup(): void {
  const on = settings.cleanup_enabled;
  cleanupOn.setAttribute("aria-checked", String(on));
  cleanupOff.setAttribute("aria-checked", String(!on));
  cleanupHint.textContent = on ? CLEANUP_HINTS.on : CLEANUP_HINTS.off;
  cleanupSelect.hidden = !on;
  if (on) cleanupPicker.render();
  else modelInput.hidden = true;
  updateResetButtons();
}

async function chooseCleanup(enabled: boolean): Promise<void> {
  if (settings.cleanup_enabled === enabled) return;
  settings = await invoke<Settings>("set_cleanup_enabled", { enabled });
  renderCleanup();
  flashSaved();
}

cleanupOn.addEventListener("click", () => void chooseCleanup(true));
cleanupOff.addEventListener("click", () => void chooseCleanup(false));

modelReset.addEventListener("click", async () => {
  await invoke<Settings>("set_cleanup_enabled", { enabled: defaults.cleanup_enabled });
  await saveCleanupModel(defaults.cleanup_model);
  renderCleanup();
});

// ---- transcription engine ------------------------------------------------

const ENGINE_HINTS: Record<Engine, string> = {
  cloud: "Runs on OpenRouter. Your audio is sent online.",
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
  cloudSelect.hidden = local;
  if (local) cloudOther.hidden = true;
  else cloudPicker.render();
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
  await invoke<Settings>("set_cloud_transcription_model", {
    model: defaults.cloud_transcription_model,
  });
  settings = await invoke<Settings>("set_local_model", { model: defaults.local_model });
  renderEngine();
  flashSaved();
});

// ---- init ----------------------------------------------------------------

async function init(): Promise<void> {
  defaults = await invoke<Settings>("get_defaults");
  settings = await invoke<Settings>("get_settings");
  renderHotkey();
  await loadKey();
  const catalog = await invoke<CloudCatalog>("list_cloud_models");
  cleanupPicker = createModelPicker({
    select: cleanupSelect,
    input: modelInput,
    options: catalog.cleanup,
    current: () => settings.cleanup_model,
    save: saveCleanupModel,
  });
  cloudPicker = createModelPicker({
    select: cloudSelect,
    input: cloudOther,
    options: catalog.transcription,
    current: () => settings.cloud_transcription_model,
    save: saveCloudModel,
  });
  renderCleanup();
  await loadLocalModels();
  renderEngine();
  await loadDevices();
}

// Microphones come and go; refresh the list whenever the window is focused.
window.addEventListener("focus", () => void loadDevices());
void init();
