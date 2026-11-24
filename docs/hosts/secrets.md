# Secrets: the shed-secrets crate and shed's own

One store implementation for shed, cooee and drover (#259, slice H1). Each program links `shed-secrets` and reads
its own secrets in its own process; shed's server keeps only the ones shed itself uses, and no shed method ever
returns a value.

## The crate

```rust
let store = shed_secrets::Store::open(Place {
    service: "cooee-secret".into(),   // macOS: the Keychain service of every item
    prefix: format!("{hash}-"),       // macOS: before each item's account name
    files: data.join("secrets"),      // elsewhere: the folder of 0600 files
});
store.set("TOKEN", &Secret::new(value)?)?;
let token = store.get("TOKEN")?;      // Option<Secret>
```

| call | what it does |
|---|---|
| `get` | macOS: the Keychain item, else a file left in the folder; elsewhere the file, one trailing newline dropped; an empty file is none |
| `set` | macOS: a Keychain item, only from a program with an Apple-issued signature; elsewhere a 0600 file in a 0700 folder, written beside and renamed in |
| `remove` | the item and the file; twice is once |
| `saved` | names and when each was saved, never a value: the Keychain's on macOS, else the folder's (dot files skipped) |
| `adopt_files` | macOS: each file into the Keychain, read back, then overwritten with zeros and removed; elsewhere nothing moves |
| `problems` | each file wider than 0600, a Keychain that does not answer, an unsigned program |

A name is `[A-Za-z0-9._-]`, 1 to 128 characters, not starting with a dot. A value is 1 to 8,192 bytes.

`Secret` has no `Debug`, `Display`, `Serialize` or `Clone` (compile-fail doc tests hold it); `.expose()` is
banned outside the store's `keychain.rs` and `files.rs`. The errors never hold a value:

| error | when |
|---|---|
| `Open` | the file is wider than 0600 and was not read; the message has the `chmod 600` |
| `Locked` | a Keychain call did not answer in 5 s (`WAIT`); every call runs on a thread of its own, prompts off |
| `Unsigned` | macOS: this program has no Apple-issued signature, so its next build could not open what it saved; nothing is saved |
| `Refused` | the Keychain refused the call |
| `Name`, `Size` | a bad name or value |
| `Io` | the folder or a file could not be read or written |

## shed's own

The server's store is the Keychain service `shed-secret` on macOS (no prefix), else `secrets/` beside the state
file. Its methods run on the main host only:

| method | params → result | |
|---|---|---|
| `secret.set` | `{name, value}` → `{}` | the value is never logged, saved in the state file or echoed in an error |
| `secret.ls` | `{}` → `[{name, saved_at}]` | names only |
| `secret.rm` | `{name}` → `{}` | twice is once |

There is no `secret.get`, and a script may not call `secret.set`. A store that refuses is `SECRET_STORE`.

```sh
shed secret set BOAT_API_KEY < key.txt   # the value from stdin, never an argument
shed secret ls                           # NAME SAVED
shed secret rm BOAT_API_KEY
```

`shed doctor` adds a `secrets` row for each file wider than 0600 and for a Keychain that does not answer (both
fail), and a note for a shed with no Apple-issued signature on macOS (a local build: it saves nothing).
