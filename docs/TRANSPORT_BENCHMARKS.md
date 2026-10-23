# Transport measurements

Measured on October 23, 2026 on Apple M4 Pro, macOS arm64, with Google Chrome
155.0.8059.86. Both CLIs used release builds and identical synthetic loopback
responses. This measures the CLI, not just the browser driver.

## Results

Each primary cohort has 20 warm samples. Times are process-spawn to process-exit,
including internal pacing. Cold samples are separate.

| Cohort | Hycli p50 / p95 (ms) | itda p50 / p95 (ms) |
|---|---:|---:|
| Direct HTTP | 20.6 / 48.0 | 19.7 / 56.1 |
| Browser fetch, default lifecycle | 2,240.4 / 3,845.8 | 115.1 / 167.5 |
| Render, default lifecycle | 3,071.2 / 5,281.2 | 115.5 / 210.3 |
| Render, Hycli attached to an already-running isolated Chrome | 1,559.0 / 1,571.7 | Same itda warm render cohort above |
| Four origins, concurrent render calls | 6,434.3 / 7,024.1 | 1,302.7 / 1,455.4 |

For the concurrent batch, both tools completed 20 reads with four workers:
Hycli took 32.251 seconds (0.620 reads/s); itda took 6.471 seconds
(3.091 reads/s). Pacing remains enabled.

The primary run completed **195/195 reads**, including cold samples, with no
cleanup failures. With a warm worker, itda's browser-fetch p95 was 23.0x lower
than Hycli's and its render p95 25.1x lower; concurrent throughput was 4.98x
higher. Most of that gap is lifecycle: a default Hycli browser call starts and
closes Chrome, itda keeps its worker. Against Hycli attached to an
already-running Chrome, itda's render p95 was 7.47x lower.

The warm numbers hold only while the worker is alive; it exits after five idle
minutes. A caller that comes back later pays the cold cost: the first direct
calls took 248 ms for Hycli and 918 ms for itda; the first browser fetch
2,966 ms and 2,792 ms; the first render 3,199 ms and 3,143 ms. These are fresh
data-directory and profile samples, not machine-cache-flushed experiments.

The aim for direct HTTP was a p95 within 5% of Hycli's. It was not met. One
predefined follow-up alternated AB/BA order for 30 pairs, without changing
binaries or resetting policy per call:

| Direct HTTP follow-up | Hycli | itda |
|---|---:|---:|
| p50 | 18.6 ms | 17.7 ms |
| p95 | 68.1 ms | 94.2 ms |
| p99 | 118.4 ms | 163.7 ms |

All **62/62 follow-up reads**, including cold samples, and cleanup succeeded.
itda's median was slightly lower, but its p95 was 16.9% higher in the primary
run and 38.4% higher in the paired run. With 20 and 30 samples, p95 and p99 are
the largest one or two values, so those percentages are rough; the medians are
the stable figure. These comparisons do not identify the cause or show a
regression from itda's pre-change version, which was not benchmarked here.

## Method

- Hycli (the project itda's `net` layer derives from; see the README's
  credits) is the unmodified official checkout at
  `192d71329720f553b382a366573ef06f4e156bb7`. itda is the
  `feature/adaptive-transport` branch as it was on the day; the data files
  carry the binary's SHA-256.
- Each cohort has its own persistent data directory. Installed definitions,
  pacing, budgets and browser state are not reset between samples.
- Interactive primary samples have equal 1,500 ms think time outside the
  measured interval. Paired samples alternate tool order with 1,500 ms between
  pairs. Concurrent samples include all pacing waits.
- Direct mode is explicit HTTP for both. itda recovery is automatic; Hycli uses
  its explicit browser transports because its localhost policy suppresses
  automatic escalation. Default Hycli browser calls launch/close the engine;
  itda retains its worker. The separate live-CDP control exposes that lifecycle
  difference rather than hiding it.
- Payload fields are checked, not just exit codes. Only correct successes enter
  latency quantiles; a failed attempt would still be counted. Quantiles use the
  nearest rank. No builds or browser QA ran during measurement.
- The script needs [Bun](https://bun.sh) (`Bun.spawn`, `Bun.serve`) and a
  locally built Hycli; it takes its flags as `--name value` pairs and ignores
  a flag it does not know.
- A CLI read is not necessarily one HTTP request. For 20 warm browser-fetch
  reads, the fixtures observed 60 total requests from Hycli and 40 from itda;
  warm render observed 40 and 20. Bootstrap/favicon requests are included.
- Results are local synthetic measurements, not a real-site success-rate study.
  Windows/Linux behavior and arbitrary website challenges were not exercised.

The full raw samples, cold results, counts and binary SHA-256 hashes are in
[the primary data](benchmarks/2026-10-23-transports.json) and
[the paired data](benchmarks/2026-10-23-direct-paired.json).
Only local binary paths were replaced with labels; measurements are unchanged.

## Reproduce

Build the pinned Hycli checkout using its build instructions, including its
dashboard assets (`npm --prefix dashboard ci`, `npm --prefix dashboard run check`,
`npm --prefix dashboard run build`) before `cargo build --release --locked --bin hycli`.
Set `HYCLI_BIN` to that executable and `CHROME` to the installed browser.

From this worktree:

```sh
cargo build --release --locked -p itda-cli
ITDA_BIN="${CARGO_TARGET_DIR:-target}/release/itda"
bun scripts/bench-transports.mjs \
  --itda "$ITDA_BIN" --hycli "$HYCLI_BIN" --browser "$CHROME" \
  --out /tmp/itda-transports.json --samples 20 --think-ms 1500 --live yes
bun scripts/bench-transports.mjs \
  --itda "$ITDA_BIN" --hycli "$HYCLI_BIN" --browser "$CHROME" \
  --out /tmp/itda-direct-paired.json --samples 30 --think-ms 1500 \
  --only direct-paired
```

## Verification

With `ITDA_TEST_BROWSER` set to the installed Chrome, workspace tests,
`cargo fmt --all --check`, strict workspace/all-target clippy, layer and
changelog checks, and the locked release build passed. Actual CLI tests cover
cross-process session reuse, concurrent origins, worker death after execution
and worker death before DevTools registration. Browser tests cover credential
changes, raw A-B-A redirect cookies, foreign resources/frames/workers, read-only
GraphQL, optional-resource cooldowns and a shutdown that leaves nothing
running.
