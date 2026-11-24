wire: secret.set
params: SecretSet

Saves one of shed's own secrets (a VM provider's key) on the main host: a Keychain item on macOS, a 0600 file elsewhere. The value is never logged, saved in the state file or returned.
