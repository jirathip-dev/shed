- `shed agents add` installs an `npx` agent once under `agents/<id>/<version>/` with `npm install` and records
  `node <its bin>`, so no `npm exec` stays resident beside the adapter; `rm` removes the install. Agents
  recorded earlier as `npx -y …` keep working; `rm` and `add` move one to the new form.
