import { invoke } from "@tauri-apps/api/core";

type CleanupState = "pending" | "done" | "failed" | "off";
type LastDictation = {
  recorded_at: number;
  audio_seconds: number;
  transcription_ms: number;
  transcription_engine: "cloud" | "local";
  transcription_model: string;
  transcription_cost_usd: number;
  transcript: string;
  cleanup: CleanupState;
  cleanup_mode: "off" | "tidy" | "polish" | null;
  cleanup_model: string | null;
  cleanup_ms: number | null;
  cleanup_cost_usd: number | null;
  cleaned: string | null;
  total_ms: number | null;
};

function wordCount(text: string): number {
  return text.split(/\s+/).filter(Boolean).length;
}

function formatMs(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

function formatCost(usd: number): string {
  if (usd === 0) return "$0.00";
  // Individual dictations cost fractions of a cent; keep them visible.
  return usd < 1 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

function formatWords(n: number): string {
  return `${n.toLocaleString()} ${n === 1 ? "word" : "words"}`;
}

function el<T extends HTMLElement>(id: string): T {
  return document.querySelector<T>(`#${id}`)!;
}

const NO_CLEANED_TEXT: Record<Exclude<CleanupState, "done">, string> = {
  pending: "Cleaning up…",
  failed: "Clean-up failed; the raw transcript was pasted.",
  off: "Clean-up was turned off for this dictation.",
};

function render(last: LastDictation | null): void {
  el("last-empty").hidden = last !== null;
  el("last-content").hidden = last === null;
  if (!last) return;

  const text = (id: string, value: string): void => {
    el(id).textContent = value;
  };
  const cleaned = last.cleaned;

  text("last-when", new Date(last.recorded_at * 1000).toLocaleString());
  text("last-total-time", last.total_ms !== null ? formatMs(last.total_ms) : "–");
  text("last-audio", `${last.audio_seconds.toFixed(1)} s`);
  text("last-t-engine", last.transcription_engine === "cloud" ? "Cloud (OpenRouter)" : "On this Mac");
  text("last-t-model", last.transcription_model);
  text("last-t-words", formatWords(wordCount(last.transcript)));
  text("last-t-time", formatMs(last.transcription_ms));
  text("last-t-cost", formatCost(last.transcription_cost_usd));
  el("last-t-text").textContent = last.transcript;

  const cleanupRan = last.cleanup_ms !== null;
  text("last-c-mode", last.cleanup_mode ? CLEANUP_MODES[last.cleanup_mode] : "–");
  text("last-c-model", last.cleanup_model ?? "–");
  text("last-c-words", cleaned !== null ? formatWords(wordCount(cleaned)) : "–");
  text(
    "last-c-time",
    cleanupRan ? formatMs(last.cleanup_ms!) : last.cleanup === "pending" ? "In progress" : "–",
  );
  text("last-c-cost", last.cleanup_cost_usd !== null ? formatCost(last.cleanup_cost_usd) : "–");
  text(
    "last-total-cost",
    formatCost(last.transcription_cost_usd + (last.cleanup_cost_usd ?? 0)),
  );

  const cleanedBox = el("last-c-text");
  cleanedBox.textContent = cleaned ?? NO_CLEANED_TEXT[last.cleanup as Exclude<CleanupState, "done">];
  cleanedBox.classList.toggle("empty", cleaned === null);
  el<HTMLButtonElement>("last-c-copy").hidden = cleaned === null;
}

async function load(): Promise<void> {
  render(await invoke<LastDictation | null>("get_last_dictation"));
}

const CLEANUP_MODES = { off: "Off", tidy: "Tidy", polish: "Polish" } as const;

let messageTimer: number | undefined;

function say(text: string, error = false): void {
  const message = el("last-message");
  message.textContent = text;
  message.classList.toggle("error", error);
  window.clearTimeout(messageTimer);
  messageTimer = window.setTimeout(() => (message.textContent = ""), 2000);
}

async function copy(which: "transcript" | "cleaned"): Promise<void> {
  try {
    await invoke("copy_last_dictation", { which });
    say("Copied to the clipboard.");
  } catch (e) {
    say(`Could not copy: ${e}`, true);
  }
}

async function run(command: string, done?: string): Promise<void> {
  try {
    await invoke(command);
    if (done) say(done);
  } catch (e) {
    say(`${e}`, true);
  }
}

el("last-reveal").addEventListener("click", () => void run("reveal_last_dictation"));
el("last-clear").addEventListener("click", () => {
  void run("clear_last_dictation").then(load);
});
el("last-t-copy").addEventListener("click", () => void copy("transcript"));
el("last-c-copy").addEventListener("click", () => void copy("cleaned"));

// Dictations happen while this window may be open behind another app, so
// refresh on focus and, while visible, every few seconds.
const REFRESH_MS = 3000;

window.addEventListener("focus", () => void load());
window.setInterval(() => {
  if (document.visibilityState === "visible") void load();
}, REFRESH_MS);
void load();
