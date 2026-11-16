- `shed kill` of a terminal running an interactive shell no longer waits out the 2 s grace: a kill sends SIGHUP
  100 ms after SIGTERM (each with SIGCONT), so a shell ends at once and `shed serve` stops quickly. A program
  that handles SIGTERM still runs its handler first; one that ignores both is still killed after the grace.
