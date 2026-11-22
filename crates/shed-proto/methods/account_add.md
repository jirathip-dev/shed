wire: account.add
params: AccountAdd
result: Account

Make an account: an empty config folder of mode 0700 under `accounts/<provider>/<name>`, or with `folder` a link to that existing folder, adopted in place; a folder holding one of the record's `setup_files` is refused (`ACCOUNT_SETUP`).
`token_file`, a file of mode 0600 on this host, is copied in as the account's setup token. The same call twice is one account.
