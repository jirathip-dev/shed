wire: code.find
params: FindParams
result: FindResult
on: machine

The files under `root` that best answer `query`'s words, best first: the top 10 (`top`, at most 50) of a stated `total`.
A word earns points in a file's name, its folders, its symbols' names and its lines; the sum is multiplied by the words that earned any and halved for a test.
Each file names its score, short `why` reasons and the symbols that earned them.
The walk and its limits are `code.grep`'s.
`machine` searches there.
