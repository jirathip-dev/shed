// ceiling: steps 11984553, peak_bytes 12236484
const lanes = []
for (let i = 0; i < 20000; i++) {
  lanes[i] = { id: `lane-${i}`, fleet: i % 7 === 0 ? "perf" : "main", idle: (i * 7919) % 1000, tags: [i % 3, i % 5] }
}
const scale = (k) => ({ id, idle, ...rest }) => ({ id, idle: idle * k, ...rest })
const rows = lanes
  .map(scale(2))
  .filter(({ fleet, tags: [a] }) => fleet === "main" && a !== 1)
  .sort((a, b) => a.idle - b.idle)
  .map(({ id, idle, tags }) => `${id}: idle ${idle}, tags ${tags}`)
return [rows.length, rows[0], rows[rows.length - 1]]
