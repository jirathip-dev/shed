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
