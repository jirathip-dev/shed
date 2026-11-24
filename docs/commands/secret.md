`shed secret set|ls|rm`

shed's own secrets on the main host (a VM provider's key): `set <NAME>` saves the value it reads from stdin, never from an argument; `ls` lists them by name with when each was saved, never a value; `rm <NAME>` removes one, and twice is once (docs/hosts/secrets.md).
