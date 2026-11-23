wire: flow.report
params: ReportParams
result: Report
on: machine

Each flow's table as it ran beside today's code (#309): its mode, the subjects it tracks, how often each row fired, the facts it never took, the `never` rows that fired and the `stuck` reasons, all kept in `flow-counts.json` since they were first kept; and the lines of `flow-shadow.jsonl` from `since` on (default the last 7 days), by class: `state` when the table and today's code showed different states for more than 2 s, `action` when the table's row would have acted and today's code did not.
`machine` asks that machine, over its held link.
