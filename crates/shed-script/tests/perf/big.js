// ceiling: steps 3818922, peak_bytes 40717337
const line = "lane-0001 blocked: waiting on the review of its change, last output 12 lines ago"
const answer = `${line}\n`.repeat(8 * 1024 * 1024 / 82)
const lines = answer.split("\n")
const out = lines.map((l, i) => `${i} ${l.slice(0, 20)}`).join("\n")
return [answer.length, lines.length, out.length]
