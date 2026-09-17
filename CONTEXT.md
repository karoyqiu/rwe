# rwe

`rwe` injects extra environment variables into a program's launch. A renamed copy of the `rwe`
binary is placed early on `PATH`, intercepts invocations of a program, applies the matching
environment file, and hands off to the real program.

## Language

**Name**:
The identifier that drives every lookup, derived from the file stem of the running binary's own
path. It selects both the target to search for and the env file to load.
_Avoid_: argv[0], program name, command

**Shim**:
A copy of the `rwe` binary renamed to a Name and placed on `PATH` ahead of the target. It is what
the user actually invokes.
_Avoid_: wrapper, proxy, launcher, stub

**Target**:
The real program the Name refers to, located on `PATH` excluding the shim itself.
_Avoid_: child, real binary, downstream program

**Env file**:
A plain-text file at `~/.config/rwe/<Name>.env` holding one `name=value` pair per line. Its
absence is normal; its presence means the target must be launched with those variables applied.
_Avoid_: dotenv, config file, profile

**Injection**:
Merging an env file's pairs over the inherited environment, with the env file winning on conflict.
_Avoid_: overlay, patch, merge
