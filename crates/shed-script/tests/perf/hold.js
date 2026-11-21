const kept = []
for (let i = 0; i < 380000; i++) {
  kept[i] = { n: i, pair: [i, i] }
}
let made = 0
for (let i = 0; i < 1400000; i++) {
  const garbage = { n: i, pair: [i, i] }
  made = made + garbage.n
}
return { kept: kept.length, made }
