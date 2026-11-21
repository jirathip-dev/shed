- `shed install`, `shed uninstall`, `shed doctor` and `--autostart` write, load, remove and read the two user
  jobs through farrier-service. On Linux the units are named after the labels (`dev.shed.serve.service`,
  `dev.shed.update.timer`) and `shed install` removes the old `shed.service` and `shed-update.*`; on macOS
  the logs are `~/Library/Logs/dev.shed.serve.log` and `dev.shed.update.log`. `shed uninstall` unloads and
  removes both jobs instead of printing the lines, and `shed install` loads only a job that changed or is
  not loaded. The job templates in `dist/` are gone: their golden copies are farrier's `fixtures/units/`.
