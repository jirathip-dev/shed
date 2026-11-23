wire: file.write
params: FileParams
on: machine

Replace a file with `data`: written to a temp file beside it, then renamed over it, its mode kept, or `mode` when given (the temp file has it before any byte is written).
`machine` writes it there.
