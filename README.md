# rwe

Run a program with extra environment variables, without a wrapper script.

Put a renamed copy of `rwe` on `PATH` ahead of the real program. When something invokes that
program, `rwe` loads `~/.config/rwe/<name>.env`, applies it, and hands off. Nothing else in the
system needs to know.

## Install

Build it, then copy the binary under the name of the program you want to wrap and put that copy
somewhere early on `PATH`:

```sh
cargo build --release

# Windows
copy target\release\rwe.exe C:\shims\node.exe

# Unix
cp target/release/rwe ~/.local/shims/node
```

Write the variables it should inject:

```sh
# ~/.config/rwe/node.env
NODE_OPTIONS=--max-old-space-size=8192
HTTPS_PROXY=http://127.0.0.1:7890
```

Now `node` picks those up wherever it is invoked from.

## Env file format

One `name=value` per line. Blank lines and `#` comments are ignored, the split is on the first
`=`, and a matched pair of surrounding double quotes is stripped from the value:

```ini
# a comment
PLAIN=value
SPACED="value with spaces"
URL=http://example.com/?a=1
EMPTY=
```

There is no variable expansion and no escape sequences — `$FOO` and `%FOO%` are literal text.
`EMPTY=` sets an empty string; there is no syntax for unsetting a variable.

The variables are applied on top of the inherited environment, and win on conflict. A repeated
name takes its last value.

## Where things live

Config lives in `~/.config/rwe/` on every platform, including Windows. `$XDG_CONFIG_HOME` is
honoured where it is set, and `$RWE_CONFIG_DIR` overrides everything.

## Behaviour

- A missing env file is not an error: `rwe` launches the program unchanged. Installing it should
  never change what happens.
- An env file that exists but cannot be read is fatal (exit `78`), because running without the
  variables would silently do the wrong thing.
- A malformed line is reported on stderr and skipped.
- The program's exit code, stdin, stdout, and stderr are passed straight through.
- `PATH` is searched in order, honouring `PATHEXT` on Windows. The current directory is never
  searched, so a binary dropped into a project folder cannot hijack the name.
- On Windows the target is held in a job object, so killing `rwe` cannot leave it orphaned. On
  Unix `rwe` `exec`s the target and disappears.

Exit codes: `127` nothing on `PATH` answers to the name, `126` the program could not be started,
`78` the env file could not be read.

## Debugging

`RWE_DEBUG=1` prints the decision trail to stderr: the resolved name, the copy of itself it
skipped, the program it chose, the env file it looked for, and the names it injected. Values are
never printed.

```
$ RWE_DEBUG=1 node --version
rwe: own path: C:\shims\node.exe
rwe: name: node
rwe: skipping myself at C:\shims\node.EXE
rwe: target: C:\Program Files\nodejs\node.EXE
rwe: C:\Users\me\.config\rwe\node.env: injecting NODE_OPTIONS, HTTPS_PROXY
rwe: args: ["--version"]
```

## License

MIT
