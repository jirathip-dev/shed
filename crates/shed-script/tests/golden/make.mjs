#!/usr/bin/env node
// Runs the worked examples beside it (the README's, in docs/architecture/script/) against a fixture host and
// writes, beside each `<name>.js`, its reply as `<name>.reply.json` and the answers its calls got, in call
// order, as `<name>.host.json`. The examples are in the subset, which is JavaScript, so Node can run them; the
// host, the guard, the clock and the reply are stand-ins shaped as the ADR says. Run it by hand when an
// example changes (`node crates/shed-script/tests/golden/make.mjs`); examples_match_their_goldens replays the
// answers through the engine and compares the reply byte for byte. CI never runs Node.
import { readFileSync, writeFileSync } from "node:fs";

const examples = ["blocked.js", "start.js", "errors.js", "refused.js"];
const here = (name) => new URL(name, import.meta.url);

const lane = (name, role, issue, reported) => ({
  name, labels: { fleet: "shed", issue, role }, cwd: `/home/user/work/shed/issue-${issue}`, cmd: ["claude"],
  keep: false, state: "running", exit: null, pid: 4000 + Number(issue), reported,
});
const lanes = [
  lane("impl-190", "impl", "190", { state: "blocked", kind: "permission", note: "Bash: cargo test" }),
  lane("rev-188", "rev", "188", { state: "working" }),
  lane("impl-191", "impl", "191", { state: "blocked", kind: "question", note: "Which cap raise?" }),
  lane("fix-185", "fix", "185", { state: "done" }),
];
const screens = {
  "impl-190": "Bash(cargo test --workspace --locked)\nDo you want to proceed?\n> 1. Yes   2. No",
  "impl-191": "The slice needs 180 more lines than the cap has.\nRaise the cap, or trim the CLI help?\n> ",
};
const history = Array.from({ length: 2913 }, (_, i) => `   Compiling crate-${i} v0.1.0`);
history[411] = "error[E0308]: mismatched types";
history[1840] = "  error[E0599]: no method named `settle` found for struct `Machine`";
history[2912] = "test result: FAILED. 212 passed; 1 failed; 0 ignored";

// One fixture call: [latency in ms, result]. The guard is one process for the run: the first guarded call
// pays its start, 20 ms; it answers the later ones in under a millisecond, 0 on this clock.
const guarded = new Set(["run", "send", "keys", "kill", "rm", "resize", "state", "acp.model", "exec", "file.write", "file.read"]);
const methods = {
  ls: (p) => [1, lanes.filter((t) => Object.entries(p.labels ?? {}).every(([k, v]) => t.labels[k] === v))],
  read: (p) => [1, { text: p.history ? history.join("\n") : screens[p.name].split("\n").slice(-p.lines).join("\n") }],
  run: (p) => [18, { ...lane(p.name, p.labels.role, p.labels.issue), cmd: p.cmd, cwd: p.cwd, idle_after: p.idle_after }],
  send: () => [4, {}],
  wait: (p) => p.until.includes("idle")
    ? [2100, { outcome: "matched", terminal: lane(p.name, "impl", "192", { state: "idle", from: "silence" }) }]
    : [11040, { outcome: "matched", terminal: lane(p.name, "impl", "192", { state: "blocked", kind: "permission", note: "Bash: git status" }) }],
};
const guard = (method) => (method === "kill" ? "lanes are stopped with drover kill" : null);

async function run(code) {
  let [now, calls, pending, guardUp] = [0, 0, [], false];
  const [logs, acting, answers] = [[], [], []];
  const tick = () => {
    pending.sort((a, b) => a.at - b.at);
    const next = pending.shift();
    if (next) {
      now = Math.max(now, next.at);
      next.settle();
    }
  };
  const call = (method, params, at) => {
    calls += 1;
    return new Promise((resolve, reject) => {
      const refused = guarded.has(method) ? guard(method) : null;
      const [ms, result] = refused === null ? methods[method](params) : [1, null];
      const entry = guarded.has(method) ? { method, params: structuredClone(params) } : null;
      if (entry) acting.push(entry);
      const settle = () => {
        if (refused === null) {
          if (entry) entry.result = structuredClone(result);
          resolve(structuredClone(result));
        } else {
          const error = { name: "GuardError", message: `${method} refused by the guard: ${refused}`, ...at, code: "GUARD_REFUSED", method };
          entry.error = { code: error.code, message: error.message };
          reject(error);
        }
      };
      const start = guarded.has(method) && !guardUp ? 20 : 0;
      guardUp ||= guarded.has(method);
      pending.push({ at: now + ms + start, settle });
      answers.push(refused === null
        ? { at: now + ms + start, result }
        : { at: now + ms + start, error: { name: "GuardError", code: "GUARD_REFUSED", message: `${method} refused by the guard: ${refused}` } });
    });
  };
  const path = (name) => new Proxy(() => {}, {
    get: (_, key) => path(name ? `${name}.${key}` : key),
    apply: (_, __, [params]) => {
      const [, line, column] = new Error().stack.split("\n")[2].match(/<anonymous>:(\d+):(\d+)/);
      return call(name, params ?? {}, { line: line - 2, column: Number(column) });
    },
  });
  String.prototype.matches = function (pattern) { return new RegExp(pattern).test(this); };
  const AsyncFunction = (async () => {}).constructor;
  const pump = setInterval(tick, 0);
  const console = { log: (...a) => logs.push(a.map((v) => (typeof v === "string" ? v : JSON.stringify(v))).join(" ")) };
  try {
    const result = (await new AsyncFunction("shed", "console", code)(path(""), console)) ?? null;
    return [{ result, logs, calls, acting, ms: now }, answers];
  } catch (error) {
    return [{ error, logs, calls, acting, ms: now }, answers];
  } finally {
    clearInterval(pump);
  }
}

for (const name of examples) {
  const [reply, answers] = await run(readFileSync(here(name), "utf8"));
  writeFileSync(here(name.replace(/\.js$/, ".reply.json")), `${JSON.stringify(reply)}\n`);
  writeFileSync(here(name.replace(/\.js$/, ".host.json")), `${JSON.stringify(answers)}\n`);
  console.log(`wrote ${name}: ${JSON.stringify(reply).length} bytes of reply, ${answers.length} answers`);
}
