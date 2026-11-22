// ceiling: steps 209909, peak_bytes 808882
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
