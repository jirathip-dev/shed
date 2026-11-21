// parsed and compiled, not run: the three worked examples repeated to 10,240 bytes
{
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
}
{
// start.js: run, send and wait for one lane in one trip
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
}
{
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
}
{
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
}
{
// start.js: run, send and wait for one lane in one trip
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
}
{
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
}
{
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
}
{
// start.js: run, send and wait for one lane in one trip
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
}
{
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
}
{
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
}
{
// start.js: run, send and wait for one lane in one trip
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
}
{
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
}
{
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
}
{
// start.js: run, send and wait for one lane in one trip
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
}
{
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
}
{
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
}
{
// start.js: run, send and wait for one lane in one trip
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
}
{
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
}
{
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
}
{
// start.js: run, send and wait for one lane in one trip
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
}
{
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
}
