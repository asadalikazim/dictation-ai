import { invoke } from "@tauri-apps/api/core";

type Summary = {
  transcription: { audio_seconds: number; words: number; cost_usd: number };
  cleanup: { words: number; cost_usd: number };
  total_cost_usd: number;
};
type Usage = { last_24h: Summary; all_time: Summary };

function formatDuration(totalSeconds: number): string {
  const s = Math.round(totalSeconds);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

function formatCost(usd: number): string {
  if (usd === 0) return "$0.00";
  // Individual dictations cost fractions of a cent; keep them visible.
  return usd < 1 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

function setCell(id: string, text: string): void {
  document.querySelector<HTMLElement>(`#${id}`)!.textContent = text;
}

async function loadUsage(): Promise<void> {
  const usage = await invoke<Usage>("get_usage");
  const fill = (suffix: string, u: Summary): void => {
    setCell(`usage-t-audio-${suffix}`, formatDuration(u.transcription.audio_seconds));
    setCell(`usage-t-words-${suffix}`, u.transcription.words.toLocaleString());
    setCell(`usage-t-cost-${suffix}`, formatCost(u.transcription.cost_usd));
    setCell(`usage-c-words-${suffix}`, u.cleanup.words.toLocaleString());
    setCell(`usage-c-cost-${suffix}`, formatCost(u.cleanup.cost_usd));
    setCell(`usage-total-${suffix}`, formatCost(u.total_cost_usd));
  };
  fill("24h", usage.last_24h);
  fill("all", usage.all_time);
}

// Dictations happen while this window may be open behind another app, so
// refresh on focus and, while visible, every few seconds.
const REFRESH_MS = 3000;

window.addEventListener("focus", () => void loadUsage());
window.setInterval(() => {
  if (document.visibilityState === "visible") void loadUsage();
}, REFRESH_MS);
void loadUsage();
