import { watch } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { cpus, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createHash } from "node:crypto";

const flags = new Map();
for (let i = 2; i < Bun.argv.length; i += 2) flags.set(Bun.argv[i], Bun.argv[i + 1]);
for (const name of ["--itda", "--hycli", "--browser", "--out"]) {
  if (!flags.get(name)) throw new Error(`Missing ${name}`);
}
const binaries = { itda: resolve(flags.get("--itda")), hycli: resolve(flags.get("--hycli")) };
const browser = resolve(flags.get("--browser"));
const samples = Number(flags.get("--samples") ?? 20);
const thinkTime = Number(flags.get("--think-ms") ?? 1500);
const pairedDirect = flags.get("--only") === "direct-paired";
if (flags.has("--only") && !pairedDirect) throw new Error("Only direct-paired is supported.");
if (!Number.isInteger(samples) || samples < 2 || samples > 40 || !Number.isFinite(thinkTime) || thinkTime < 0) {
  throw new Error("Use 2-40 samples and a nonnegative think time.");
}
const root = await mkdtemp(join(tmpdir(), "itda-transport-bench-"));
const servers = [];
const contexts = [];
const results = [];
const cleanupFailures = [];
const counts = { itda: 0, hycli: 0, other: 0 };
const logger = (text) => process.stderr.write(`${text}\n`);
const quantile = (values, p) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.ceil(sorted.length * p) - 1)] ?? null;
};
const think = () => thinkTime ? new Promise(resolve => setTimeout(resolve, thinkTime)) : Promise.resolve();

async function execute(args, env, timeout = 90_000) {
  const start = performance.now();
  const child = Bun.spawn(args, { env, stdout: "pipe", stderr: "pipe", stdin: "ignore" });
  let expired = false;
  const timer = setTimeout(() => { expired = true; child.kill(); }, timeout);
  try {
    const [code, stdout, stderr] = await Promise.all([
      child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ]);
    return { code, stdout, stderr, elapsed_ms: performance.now() - start, expired };
  } finally {
    clearTimeout(timer);
  }
}

function fixture() {
  const server = Bun.serve({
    hostname: "127.0.0.1", port: 0,
    fetch(request) {
      const tool = request.headers.get("x-bench-tool");
      counts[Object.hasOwn(counts, tool) ? tool : "other"] += 1;
      const path = new URL(request.url).pathname;
      if (path === "/render") {
        return new Response(
          `<html><head><title>Fixture</title></head><body><main></main><p>${"Fixture content. ".repeat(40)}</p>` +
          `<script>const h=document.createElement('h1');h.id='done';h.textContent='rendered';document.querySelector('main').append(h);</script></body></html>`,
          { headers: { "content-type": "text/html; charset=utf-8" } },
        );
      }
      if (path === "/guarded" && !request.headers.has("sec-fetch-mode")) {
        return new Response("<title>Just a moment</title>", { status: 403, headers: { "content-type": "text/html" } });
      }
      if (path === "/api" || path === "/guarded") {
        return Response.json({ ok: true, items: [{ id: 1, title: "value" }] });
      }
      return new Response("<html><body>Browser bootstrap fixture.</body></html>", {
        headers: { "content-type": "text/html" },
      });
    },
  });
  servers.push(server);
  return server.url.origin;
}

function specification(tool, name, origin) {
  return {
    spec_version: 1, site: { name, base_url: origin },
    defaults: { headers: { "X-Bench-Tool": tool } },
    operations: ["direct", "browser-fetch", "render"].map(mode => ({
      name: mode, method: "GET", effect: "read", evidence: "Synthetic local benchmark fixture",
      path: mode === "direct" ? "/api" : mode === "render" ? "/render" : "/guarded",
      transport: tool === "itda" || mode === "direct" ? "http" : mode === "render" ? "browser" : "browser-fetch",
      response: mode === "render" ? {
        format: "html", required_html_fields: [{ selector: "main #done" }],
        html: { items: "main", fields: { title: { selector: "#done", required: true } } },
      } : { format: "json", required_pointers: ["/ok", "/items/0/id"] },
    })),
  };
}

async function context(tool, label, origins, cdp) {
  const home = join(root, `${tool}-${label}`);
  await mkdir(home, { recursive: true, mode: 0o700 });
  const env = { ...process.env };
  for (const key of ["HYCLI_CDP", "CHROME_CDP", "HYCLI_SPECS", "HYCLI_KB"]) delete env[key];
  if (tool === "itda") Object.assign(env, {
    ITDA_HOME: home, ITDA_ALLOW_LOCAL: "1", ITDA_SECRET_STORE: "env",
    ITDA_TRANSPORT: label.startsWith("direct") ? "http" : "auto", ITDA_BROWSER: browser,
  });
  else {
    Object.assign(env, {
      HYCLI_DATA_DIR: join(home, "data"), HYCLI_TEST_DATA: join(home, "browser-cache"),
      HYCLI_DISABLE_HEADED: "1", CHROME: browser, TMPDIR: join(home, "tmp"),
    });
    for (const path of [env.HYCLI_DATA_DIR, env.HYCLI_TEST_DATA, env.TMPDIR]) await mkdir(path, { recursive: true });
    if (cdp) env.HYCLI_CDP = cdp;
  }
  for (let i = 0; i < origins.length; i++) {
    const name = `bench-${i}`;
    const spec = specification(tool, name, origins[i]);
    if (tool === "itda") {
      const dir = join(home, "specs", name);
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, "spec.yaml"), JSON.stringify(spec));
    } else {
      const path = join(home, `${name}.yaml`);
      await writeFile(path, JSON.stringify(spec));
      const installed = await execute([binaries.hycli, "--allow-local", "--format", "json", "spec", "install", path], env);
      if (installed.code !== 0) throw new Error(installed.stderr || installed.stdout);
    }
  }
  const result = { tool, label, home, env };
  contexts.push(result);
  return result;
}

async function sample(ctx, mode, site = 0, cold = false) {
  const args = ctx.tool === "itda"
    ? [binaries.itda, "call", `bench-${site}.${mode}`, "--json"]
    : [binaries.hycli, "--allow-local", "--format", "json", "run", `bench-${site}`, mode];
  const result = await execute(args, ctx.env);
  let value;
  try { value = JSON.parse(result.stdout); } catch { value = null; }
  const data = value?.data ?? value;
  const correct = data?.items?.[0]?.title === (mode === "render" ? "rendered" : "value");
  const row = {
    tool: ctx.tool, cohort: ctx.label, cold, site,
    elapsed_ms: result.elapsed_ms, success: result.code === 0 && correct,
    exit_code: result.code, timeout: result.expired,
  };
  if (!row.success) row.error = (result.stderr || result.stdout).slice(0, 2048);
  results.push(row);
  return row;
}

async function liveBrowser() {
  const profile = join(root, "live-profile");
  await mkdir(profile);
  let child;
  const address = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { observer.close(); reject(new Error("CDP startup timed out")); }, 15_000);
    const observer = watch(profile, async (_, file) => {
      if (String(file) !== "DevToolsActivePort") return;
      try {
        const text = await readFile(join(profile, "DevToolsActivePort"), "utf8");
        const [port, path] = text.trim().split("\n");
        if (!/^\d+$/.test(port) || !path?.startsWith("/devtools/browser/")) return;
        clearTimeout(timer);
        observer.close();
        resolve(`http://127.0.0.1:${port}`);
      } catch (error) {
        if (error.code !== "ENOENT") { clearTimeout(timer); observer.close(); reject(error); }
      }
    });
    child = Bun.spawn([browser, "--headless=new", "--no-first-run", "--no-proxy-server",
      "--remote-debugging-port=0", "--remote-allow-origins=http://127.0.0.1", `--user-data-dir=${profile}`],
    { stdout: "ignore", stderr: "ignore", stdin: "ignore" });
  }).catch(async error => {
    if (child) { child.kill(); await child.exited; }
    throw error;
  });
  return { address, close: async () => { child.kill(); await child.exited; } };
}

let live;
const batches = [];
const browserVersion = await execute([browser, "--version"], { ...process.env });
if (browserVersion.code !== 0) throw new Error(browserVersion.stderr);
try {
  if (pairedDirect) {
    const origin = fixture();
    const pair = [
      await context("hycli", "direct-paired", [origin]),
      await context("itda", "direct-paired", [origin]),
    ];
    for (const ctx of pair) await sample(ctx, "direct", 0, true);
    const before = { ...counts };
    for (let i = 0; i < samples; i++) {
      await think();
      for (const ctx of i % 2 ? [...pair].reverse() : pair) await sample(ctx, "direct");
    }
    for (const ctx of pair) {
      batches.push({ tool: ctx.tool, cohort: ctx.label, wire_requests: counts[ctx.tool] - before[ctx.tool] });
    }
  } else {
    for (const mode of ["direct", "browser-fetch", "render"]) {
      const origin = fixture();
      for (const tool of ["hycli", "itda"]) {
        logger(`${tool}: ${mode}, ${samples} samples`);
        const ctx = await context(tool, mode, [origin]);
        await sample(ctx, mode, 0, true);
        const before = { ...counts };
        for (let i = 0; i < samples; i++) { await think(); await sample(ctx, mode); }
        batches.push({ tool, cohort: mode, wire_requests: counts[tool] - before[tool], unattributed_requests: counts.other - before.other });
      }
    }
    if (flags.get("--live") === "yes") {
      live = await liveBrowser();
      const ctx = await context("hycli", "render-live-cdp", [fixture()], live.address);
      await sample(ctx, "render", 0, true);
      for (let i = 0; i < samples; i++) { await think(); await sample(ctx, "render"); }
    }
    const origins = Array.from({ length: 4 }, fixture);
    for (const tool of ["hycli", "itda"]) {
      const ctx = await context(tool, "parallel-render", origins);
      await Promise.all(origins.map((_, i) => sample(ctx, "render", i, true)));
      const started = performance.now();
      const before = results.length;
      await Promise.all(origins.map(async (_, site) => {
        for (let i = 0; i < Math.ceil(samples / origins.length); i++) await sample(ctx, "render", site);
      }));
      const elapsed_ms = performance.now() - started;
      const rows = results.slice(before);
      batches.push({ tool, cohort: "parallel-render", elapsed_ms, requests: rows.length,
        success: rows.filter(r => r.success).length, requests_per_second: rows.length * 1000 / elapsed_ms });
    }
  }
} finally {
  for (const ctx of contexts.filter(c => c.tool === "itda")) {
    const stopped = await execute([binaries.itda, "__browser-stop"], ctx.env, 60_000);
    if (stopped.code !== 0) {
      cleanupFailures.push({ cohort: ctx.label, error: stopped.stderr || stopped.stdout });
      logger(`Broker cleanup failed for ${ctx.label}: ${stopped.stderr}`);
    }
  }
  if (live) await live.close();
  for (const server of servers) server.stop(true);
}

const groups = [...new Set(results.filter(r => !r.cold).map(r => `${r.tool}|${r.cohort}`))].map(key => {
  const rows = results.filter(r => !r.cold && `${r.tool}|${r.cohort}` === key);
  const passed = rows.filter(r => r.success).map(r => r.elapsed_ms);
  return { key, count: rows.length, failures: rows.length - passed.length,
    p50_ms: quantile(passed, 0.5), p95_ms: quantile(passed, 0.95), p99_ms: quantile(passed, 0.99) };
});
const hashes = Object.fromEntries(await Promise.all(Object.entries(binaries).map(async ([name, path]) =>
  [name, createHash("sha256").update(await readFile(path)).digest("hex")])));
const report = {
  date: new Date().toISOString(), platform: `${process.platform}-${process.arch}`,
  binaries, hashes, browser, browser_version: browserVersion.stdout.trim(),
  cpu: cpus()[0]?.model, samples, think_time_ms: thinkTime,
  methodology: pairedDirect
    ? "Direct-only follow-up alternates AB/BA order within each pair, with equal think time between pairs. Each tool retains one home and policy database. Latency is spawn-to-exit; cold samples and all failures are retained separately."
    : "Interactive latency is spawn-to-exit including internal pacing, after equal think time. Parallel throughput includes pacing. Default CLI compares product defaults; optional live CDP isolates Hycli warm-engine performance. Cold samples are separate; only correct successes enter latency quantiles, failures remain counted.",
  groups, batches, results, cleanup_failures: cleanupFailures,
};
await mkdir(dirname(resolve(flags.get("--out"))), { recursive: true });
await writeFile(resolve(flags.get("--out")), JSON.stringify(report, null, 2));
await rm(root, { recursive: true, force: true });
process.stdout.write(`${JSON.stringify(groups, null, 2)}\n`);
if (results.some(row => !row.success) || cleanupFailures.length) process.exitCode = 1;
