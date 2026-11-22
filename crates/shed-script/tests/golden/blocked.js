// ceiling: steps 300, peak_bytes 7275
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
