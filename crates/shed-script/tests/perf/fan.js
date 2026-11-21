// ceiling: steps 44018, peak_bytes 731820
// fan.js: 2,000 calls, 32 in flight, answered at once: the engine's own cost of a call
const calls = []
for (let i = 0; i < 2000; i++) calls[i] = shed.get({ name: `t${i}` })
const got = await Promise.all(calls)
return got.length
