wire: file.read
params: FileParams
result: Vec<u8>
on: machine

A file's bytes, or with `lines` only that span's; over `REPLY_LIMIT` bytes is `IO`.
`machine` reads it there.
