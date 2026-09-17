# Always wait for the target

On Unix the shim `exec`s the target and disappears, so there is nothing to decide. On Windows there
is no `exec`: the shim must spawn the target and then choose whether to wait for it or exit
immediately. We always wait, propagate the target's exit code, and keep the target in a job object
with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so it cannot outlive the shim.

## Considered options

Exiting immediately after spawn was considered, since it is the right behaviour for GUI targets
(the terminal is freed at once). It was rejected because it loses the target's exit code, returns
the shell prompt while the target is still writing to the same console, and breaks `shim && next`
in scripts. A third option — parsing the target PE header's `Subsystem` field and waiting only for
console targets, mirroring what `cmd.exe` does — was rejected as unnecessary: the targets in
practice are command-line tools.

## Consequences

Wrapping a GUI program will occupy the calling terminal until that program exits. If that becomes a
real need, reopen this decision and implement the PE `Subsystem` check rather than adding a
configuration switch — the platform already encodes the answer.

The shim must ignore Ctrl-C rather than die on it. Windows delivers console control events to every
process in the console process group, so the target receives its own; if the shim also died it
would return the prompt while the target was still shutting down.
