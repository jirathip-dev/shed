wire: file.write
params: FileParams
on: machine

Replace a file with `data`: written to a temp file beside it, then renamed over it, its mode kept.
`machine` writes it there.
