- `shed code find [--top N] [--hidden] [-g GLOB]… 'WORDS' [PATH]` (`code.find`, with `machine` over a held
  link): the files under a folder that best answer the words, best first, the top 10 (at most 50) of a stated
  total. Each names its score, short reasons (`name is pattern`, `path has lane`, `lines: wait 89`, `a test`)
  and at most 5 symbols whose name holds a word. The walk and its limits are `code grep`'s.
