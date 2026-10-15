#!/usr/bin/env node
// Scores the clean-up prompts against a real model.
//
// It sends each case in evals/cleanup/cases.json the way the app does: the
// mode's system prompt, its worked examples as earlier chat turns, then the
// transcript wrapped in <transcript> tags. The prompt and example files are
// read from src-tauri/src, so what is tested is what ships.
//
// The checks are deterministic (substrings, patterns, line counts, length
// ratios). They catch regressions in formatting and in obeying dictated
// instructions, and they spot-check that key facts survive. They cannot judge
// whether a rewrite kept every nuance of the meaning; read the output of a
// few Polish cases with --verbose after changing that prompt.
//
// Usage:
//   node scripts/eval-cleanup.mjs [--mode tidy|polish] [--filter text]
//        [--model id] [--runs n] [--verbose]
//
// The OpenRouter key comes from OPENROUTER_API_KEY, or else from the macOS
// Keychain item the app uses (macOS may ask you to allow that).

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const SRC = join(root, "src-tauri", "src");
const MODES = ["tidy", "polish"];
const DEFAULT_MODEL = "anthropic/claude-haiku-5.5";
// Same request parameters as src-tauri/src/cleanup.rs.
const TEMPERATURE = 0.2;
const MAX_TOKENS = 2048;
const CONCURRENCY = 4;

// ---- arguments ------------------------------------------------------------

function parseArgs(argv) {
  const opts = { mode: null, filter: null, model: DEFAULT_MODEL, runs: 1, verbose: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const value = () => {
      if (i + 1 >= argv.length) fail(`${a} needs a value`);
      return argv[++i];
    };
    if (a === "--mode") opts.mode = value();
    else if (a === "--filter") opts.filter = value();
    else if (a === "--model") opts.model = value();
    else if (a === "--runs") opts.runs = Number(value());
    else if (a === "--verbose" || a === "-v") opts.verbose = true;
    else fail(`unknown argument: ${a}`);
  }
  if (opts.mode && !MODES.includes(opts.mode)) fail(`--mode must be one of ${MODES.join(", ")}`);
  if (!Number.isInteger(opts.runs) || opts.runs < 1) fail("--runs must be a positive integer");
  return opts;
}

function fail(message) {
  console.error(message);
  process.exit(2);
}

// ---- key ------------------------------------------------------------------

function apiKey() {
  const fromEnv = process.env.OPENROUTER_API_KEY?.trim();
  if (fromEnv) return fromEnv;
  try {
    return execFileSync(
      "security",
      ["find-generic-password", "-s", "com.asadkazim.dictationai", "-a", "openrouter-api-key", "-w"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    ).trim();
  } catch {
    return fail("No key: set OPENROUTER_API_KEY, or save one in the app's Settings.");
  }
}

// ---- requests -------------------------------------------------------------

function loadAssets(mode) {
  return {
    prompt: readFileSync(join(SRC, `cleanup_${mode}_prompt.txt`), "utf8"),
    examples: JSON.parse(readFileSync(join(SRC, `cleanup_${mode}_examples.json`), "utf8")),
  };
}

const wrap = (transcript) => `<transcript>${transcript}</transcript>`;

function buildMessages(assets, transcript) {
  const messages = [{ role: "system", content: assets.prompt }];
  for (const e of assets.examples) {
    messages.push({ role: "user", content: wrap(e.transcript) });
    messages.push({ role: "assistant", content: e.cleaned });
  }
  messages.push({ role: "user", content: wrap(transcript) });
  return messages;
}

async function clean(key, model, assets, transcript) {
  const body = {
    model,
    temperature: TEMPERATURE,
    max_tokens: MAX_TOKENS,
    provider: { data_collection: "deny" },
    messages: buildMessages(assets, transcript),
  };
  let lastError = "";
  for (let attempt = 0; attempt < 3; attempt++) {
    const started = Date.now();
    try {
      const res = await fetch("https://openrouter.ai/api/v1/chat/completions", {
        method: "POST",
        headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(30_000),
      });
      if (res.ok) {
        const json = await res.json();
        return {
          text: (json.choices?.[0]?.message?.content ?? "").trim(),
          cost: json.usage?.cost ?? 0,
          ms: Date.now() - started,
        };
      }
      lastError = `HTTP ${res.status}`;
      if (res.status < 500 && res.status !== 429) break;
    } catch (e) {
      lastError = String(e);
    }
    await new Promise((r) => setTimeout(r, 1000 * 2 ** attempt));
  }
  return { text: "", cost: 0, ms: 0, error: lastError };
}

// ---- checks ---------------------------------------------------------------

const NUMBERED = /^\s*\d+[.)]\s+\S/;
const BULLET = /^\s*[-*•]\s+\S/;

function stats(text) {
  const lines = text.split("\n");
  return {
    numbered: lines.filter((l) => NUMBERED.test(l)).length,
    bullets: lines.filter((l) => BULLET.test(l)).length,
    paragraphs: text.split(/\n\s*\n/).filter((p) => p.trim()).length,
  };
}

/** Checks applied to every case, whatever its own expectations. */
function universalProblems(input, out) {
  const problems = [];
  if (!out) problems.push("empty output");
  if (/^(here is|here's|sure|certainly|of course|cleaned|rewritten)\b/i.test(out)) {
    problems.push("starts with a preamble");
  }
  if (/^["“].*["”]$/s.test(out)) problems.push("wrapped in quotation marks");
  if (/\*\*|^#{1,6}\s|```/m.test(out)) problems.push("contains markdown");
  if (/<\/?transcript>/i.test(out) && !/<\/?transcript>/i.test(input)) {
    problems.push("echoes the transcript tags");
  }
  return problems;
}

function check(input, out, expect) {
  const problems = universalProblems(input, out);
  const s = stats(out);
  const lower = out.toLowerCase();
  const ratio = out.length / Math.max(1, input.length);

  if (expect.equals !== undefined && out !== expect.equals) {
    problems.push(`expected exactly ${JSON.stringify(expect.equals)}`);
  }
  for (const t of expect.contains ?? []) {
    if (!lower.includes(t.toLowerCase())) problems.push(`missing ${JSON.stringify(t)}`);
  }
  for (const t of expect.not_contains ?? []) {
    if (lower.includes(t.toLowerCase())) problems.push(`should not contain ${JSON.stringify(t)}`);
  }
  for (const p of expect.matches ?? []) {
    if (!new RegExp(p, "im").test(out)) problems.push(`no match for /${p}/`);
  }
  for (const p of expect.not_matches ?? []) {
    if (new RegExp(p, "im").test(out)) problems.push(`unwanted match for /${p}/`);
  }
  if (expect.numbered_items_min !== undefined && s.numbered < expect.numbered_items_min) {
    problems.push(`${s.numbered} numbered items, wanted ${expect.numbered_items_min}+`);
  }
  if (expect.bullet_items_min !== undefined && s.bullets < expect.bullet_items_min) {
    problems.push(`${s.bullets} bullets, wanted ${expect.bullet_items_min}+`);
  }
  if (expect.min_list_items !== undefined && s.numbered + s.bullets < expect.min_list_items) {
    problems.push(`${s.numbered + s.bullets} list items, wanted ${expect.min_list_items}+`);
  }
  if (expect.no_list && s.numbered + s.bullets > 0) problems.push("made a list");
  if (expect.paragraphs_min !== undefined && s.paragraphs < expect.paragraphs_min) {
    problems.push(`${s.paragraphs} paragraphs, wanted ${expect.paragraphs_min}+`);
  }
  if (expect.paragraphs_max !== undefined && s.paragraphs > expect.paragraphs_max) {
    problems.push(`${s.paragraphs} paragraphs, wanted at most ${expect.paragraphs_max}`);
  }
  if (expect.max_ratio !== undefined && ratio > expect.max_ratio) {
    problems.push(`length ratio ${ratio.toFixed(2)} above ${expect.max_ratio}`);
  }
  if (expect.min_ratio !== undefined && ratio < expect.min_ratio) {
    problems.push(`length ratio ${ratio.toFixed(2)} below ${expect.min_ratio}`);
  }
  return problems;
}

// ---- running --------------------------------------------------------------

async function pool(items, worker) {
  const results = new Array(items.length);
  let next = 0;
  await Promise.all(
    Array.from({ length: Math.min(CONCURRENCY, items.length) }, async () => {
      while (next < items.length) {
        const i = next++;
        results[i] = await worker(items[i]);
      }
    }),
  );
  return results;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const key = apiKey();
  const cases = JSON.parse(readFileSync(join(root, "evals", "cleanup", "cases.json"), "utf8"));

  const jobs = [];
  for (const c of cases) {
    if (opts.filter && !`${c.id} ${c.category}`.includes(opts.filter)) continue;
    const modes = c.mode === "both" || !c.mode ? MODES : [c.mode];
    for (const mode of modes) {
      if (opts.mode && opts.mode !== mode) continue;
      const expect = { ...c.expect, ...(c[`expect_${mode}`] ?? {}) };
      jobs.push({ c, mode, expect });
    }
  }
  if (jobs.length === 0) fail("no cases match");

  const assets = Object.fromEntries(MODES.map((m) => [m, loadAssets(m)]));
  console.log(
    `${jobs.length} case runs x ${opts.runs} on ${opts.model} (${[...new Set(jobs.map((j) => j.mode))].join(", ")})\n`,
  );

  const results = await pool(jobs, async (job) => {
    const runs = [];
    for (let r = 0; r < opts.runs; r++) {
      const res = await clean(key, opts.model, assets[job.mode], job.c.input);
      const problems = res.error ? [`request failed: ${res.error}`] : check(job.c.input, res.text, job.expect);
      runs.push({ ...res, problems });
    }
    return { ...job, runs };
  });

  let cost = 0;
  const times = [];
  const byMode = {};
  const failures = [];
  for (const r of results) {
    const passed = r.runs.filter((x) => x.problems.length === 0).length;
    const ok = passed === r.runs.length;
    byMode[r.mode] ??= { pass: 0, total: 0 };
    byMode[r.mode].total++;
    if (ok) byMode[r.mode].pass++;
    else failures.push(r);
    for (const run of r.runs) {
      cost += run.cost;
      if (run.ms) times.push(run.ms);
    }
    const flaky = !ok && passed > 0 ? ` (${passed}/${r.runs.length} runs)` : "";
    console.log(`${ok ? "PASS" : "FAIL"}  ${r.mode.padEnd(6)} ${r.c.category.padEnd(13)} ${r.c.id}${flaky}`);
    if (opts.verbose || !ok) {
      for (const run of r.runs) {
        if (opts.verbose || run.problems.length) {
          console.log(`        in : ${JSON.stringify(r.c.input)}`);
          console.log(`        out: ${JSON.stringify(run.text)}`);
        }
        for (const p of run.problems) console.log(`        - ${p}`);
        if (!opts.verbose) break;
      }
    }
  }

  times.sort((a, b) => a - b);
  const median = times.length ? times[Math.floor(times.length / 2)] : 0;
  console.log("");
  for (const [mode, s] of Object.entries(byMode)) {
    console.log(`${mode.padEnd(7)} ${s.pass}/${s.total} passed`);
  }
  console.log(`median latency ${median} ms, total cost $${cost.toFixed(4)}`);
  process.exit(failures.length ? 1 : 0);
}

await main();
