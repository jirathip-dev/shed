// ceiling: steps 5496713, peak_bytes 35
const add = (a, b) => a + b
let sum = 0
for (let i = 0; i < 263000; i++) {
  sum = add(sum, i % 7)
}
return sum
