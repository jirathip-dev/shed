#!/usr/bin/env node
// Runs the worked examples of README.md against a fixture host and compares each reply with the one the
// README shows. The examples are in the subset, which is JavaScript, so Node can run them; the host, the
// guard, the clock and the reply are stand-ins shaped as the ADR says. `node check.mjs` prints the replies
// and exits 1 when one differs.
import { readFileSync } from "node:fs";

const readme = readFileSync(new URL("README.md", import.meta.url), "utf8");
const examples = [...readme.matchAll(/```js\n(\/\/ (\S+\.js)[^\n]*\n[\s\S]*?)```\s*```json\n([\s\S]*?)```/g)];

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

// One fixture call: [latency in ms, result]. A guarded method pays the guard's 20 ms first.
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
  let [now, calls, pending] = [0, 0, []];
  const logs = [];
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
      const settle = () => refused === null
        ? resolve(structuredClone(result))
        : reject({ name: "GuardError", message: `${method} refused by the guard: ${refused}`, ...at, code: "GUARD_REFUSED", method });
      pending.push({ at: now + ms + (guarded.has(method) ? 20 : 0), settle });
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
    return { result, logs, calls, ms: now };
  } catch (error) {
    return { error, logs, calls, ms: now };
  } finally {
    clearInterval(pump);
  }
}

let failed = false;
for (const [, code, name, shown] of examples) {
  const reply = await run(code);
  const same = JSON.stringify(reply) === JSON.stringify(JSON.parse(shown));
  failed ||= !same;
  console.log(`${same ? "ok  " : "DIFF"} ${name}\n${JSON.stringify(reply)}\n`);
}
if (examples.length !== 4) {
  console.log(`expected 4 examples in README.md, found ${examples.length}`);
  failed = true;
}
process.exit(failed ? 1 : 0);
