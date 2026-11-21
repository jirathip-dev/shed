const add = (a, b) => a + b
let sum = 0
for (let i = 0; i < 250000; i++) {
  sum = add(sum, i % 7)
}
return sum
