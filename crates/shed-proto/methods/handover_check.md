wire: handover.check
params: HandoverCheckParams
result: Handover

Would the server hand over to the binary at `candidate`: it runs `<candidate> serve --preflight`, which must name `build`, read the state file and inherit this server's handover format, and speak the protocol of every live ACP bridge.
Changes nothing; `HANDOVER_REFUSED` says why not.
