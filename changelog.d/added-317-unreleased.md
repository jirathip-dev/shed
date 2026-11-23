- The flow tables run in shadow beside today's code: where they disagree, a line goes to `flow-shadow.jsonl`
  beside the state file; `shed flow report` (`flow.report`) shows rows fired, facts never seen, stuck rows and
  the disagreements by class, and `shed doctor` counts the last 24 h's.
