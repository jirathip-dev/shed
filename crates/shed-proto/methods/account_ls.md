wire: account.ls
params: AccountsParams
result: Vec<Account>

The accounts, per provider `default` first, then in the order `auto` picks: what shed last read of each, its token's dates (never the token), the setup files its folder holds and the terminals on it. A reading older than the record's `usage.fresh` is read again in the background.
