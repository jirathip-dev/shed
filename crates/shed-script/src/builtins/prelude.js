function map(xs, f) {
  const out = []
  for (let i = 0; i < xs.length; i++) out[i] = f(xs[i], i, xs)
  return out
}

function filter(xs, f) {
  const out = []
  for (let i = 0; i < xs.length; i++) if (f(xs[i], i, xs)) out[out.length] = xs[i]
  return out
}

function forEach(xs, f) {
  for (let i = 0; i < xs.length; i++) f(xs[i], i, xs)
}

function find(xs, f) {
  for (let i = 0; i < xs.length; i++) if (f(xs[i], i, xs)) return xs[i]
}

function findIndex(xs, f) {
  for (let i = 0; i < xs.length; i++) if (f(xs[i], i, xs)) return i
  return -1
}

function some(xs, f) {
  for (let i = 0; i < xs.length; i++) if (f(xs[i], i, xs)) return true
  return false
}

function every(xs, f) {
  for (let i = 0; i < xs.length; i++) if (!f(xs[i], i, xs)) return false
  return true
}

function reduce(xs, f, ...start) {
  let i = 0
  let acc = start[0]
  if (start.length === 0) {
    if (xs.length === 0) throw { name: "TypeError", message: "reduce of an empty array needs a start value" }
    acc = xs[0]
    i = 1
  }
  for (; i < xs.length; i++) acc = f(acc, xs[i], i, xs)
  return acc
}

function flatMap(xs, f) {
  return xs.map(f).flat()
}

function sort(xs, f) {
  const by = f ?? ((a, b) => (a < b ? -1 : b < a ? 1 : 0))
  let from = xs.slice()
  for (let width = 1; width < from.length; width *= 2) {
    const to = []
    for (let lo = 0; lo < from.length; lo += 2 * width) {
      const mid = Math.min(lo + width, from.length)
      const hi = Math.min(lo + 2 * width, from.length)
      let i = lo
      let j = mid
      while (i < mid || j < hi) {
        if (j >= hi || (i < mid && by(from[i], from[j]) <= 0)) {
          to[to.length] = from[i]
          i++
        } else {
          to[to.length] = from[j]
          j++
        }
      }
    }
    from = to
  }
  for (let k = 0; k < from.length; k++) xs[k] = from[k]
  return xs
}

async function then(p, f, g) {
  let v
  try {
    v = await p
  } catch (e) {
    if (g == null) throw e
    return g(e)
  }
  return f == null ? v : f(v)
}

function allSettled(xs) {
  return Promise.all(
    xs.map(async (p) => {
      try {
        return { status: "fulfilled", value: await p }
      } catch (e) {
        return { status: "rejected", reason: e }
      }
    }),
  )
}
