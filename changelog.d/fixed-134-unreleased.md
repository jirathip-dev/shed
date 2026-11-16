- An out-of-memory kill of one terminal's process no longer stops the systemd unit and every terminal:
  the unit `shed install` writes has `OOMPolicy=continue` (`shed install --force` updates an older one).
  `--autostart` starts the installed job instead of a server outside it, and `shed doctor` flags a server
  that answers while the job is not loaded.
