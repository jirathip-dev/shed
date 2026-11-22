- A run on another machine can use an account: shed gives it a key for that one run, never the token, and the
  token proxy on the main host signs its calls; `shed proxy keys|revoke` list and stop the keys, and
  `shed account login --token` saves an account's setup token from the login screen.
