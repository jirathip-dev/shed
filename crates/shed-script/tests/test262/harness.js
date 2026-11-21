const Test262Error = { name: "Test262Error" }
const TypeError = { name: "TypeError" }
const ReferenceError = { name: "ReferenceError" }
const RangeError = { name: "RangeError" }
const SyntaxError = { name: "SyntaxError" }
const show = (v) => typeof v === "string" ? `"${v}"` : typeof v === "object" && v !== null ? "an object" : typeof v === "function" ? "a function" : `${v}`
const fail = (message) => ({ name: "Test262Error", message: message ?? "" })
const isSame = (a, b) => a === b ? a !== 0 || 1 / a === 1 / b : a !== a && b !== b
const assert = {
  ok: (value, message) => {
    if (value !== true) throw fail(`expected true, got ${show(value)}: ${message ?? ""}`)
  },
  sameValue: (actual, expected, message) => {
    if (!isSame(actual, expected)) throw fail(`expected ${show(expected)}, got ${show(actual)}: ${message ?? ""}`)
  },
  notSameValue: (actual, unexpected, message) => {
    if (isSame(actual, unexpected)) throw fail(`expected anything but ${show(unexpected)}: ${message ?? ""}`)
  },
  throws: (expected, f, message) => {
    try {
      f()
    } catch (e) {
      if (e === null || typeof e !== "object" || e.name !== expected.name) {
        throw fail(`expected a ${expected.name}, got ${show(e?.name ?? e)}: ${message ?? ""}`)
      }
      return
    }
    throw fail(`expected a ${expected.name} to be thrown: ${message ?? ""}`)
  },
}
const $DONOTEVALUATE = () => {
  throw fail("this test should not have been evaluated")
}
