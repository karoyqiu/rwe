//! rwe — run a program with extra environment variables.
//!
//! A copy of this binary renamed to `<name>` and placed on `PATH` ahead of the real `<name>`
//! intercepts the invocation, applies `~/.config/rwe/<name>.env`, and hands off to the real
//! program. See `CONTEXT.md` for the vocabulary and `docs/adr/` for the decisions.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Nothing on `PATH` answers to the name, or we would have re-entered ourselves.
const EXIT_NOT_FOUND: i32 = 127;
/// A target was found but could not be started.
const EXIT_SPAWN_FAILED: i32 = 126;
/// An env file exists but could not be read (`sysexits.h` `EX_CONFIG`).
const EXIT_CONFIG: i32 = 78;
/// We could not work out our own identity (`sysexits.h` `EX_SOFTWARE`).
const EXIT_INTERNAL: i32 = 70;

fn debug_enabled() -> bool {
    std::env::var_os("RWE_DEBUG").is_some_and(|v| !v.is_empty())
}

macro_rules! trace {
    ($($arg:tt)*) => {
        if debug_enabled() {
            eprintln!("rwe: {}", format_args!($($arg)*));
        }
    };
}

fn fail(code: i32, message: std::fmt::Arguments) -> ! {
    eprintln!("rwe: {message}");
    std::process::exit(code)
}

fn main() {
    let own_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => fail(
            EXIT_INTERNAL,
            format_args!("cannot determine own path: {err}"),
        ),
    };
    trace!("own path: {}", own_path.display());

    // The name comes from our own path, never from argv[0]: the caller controls argv[0] and we
    // use the name for three things at once (recursion guard, target lookup, env file lookup).
    let Some(name) = name_of(&own_path) else {
        fail(
            EXIT_INTERNAL,
            format_args!("cannot derive a name from {}", own_path.display()),
        )
    };
    trace!("name: {name}");

    // Guard per name, not globally: a shim for `a` launching a shim for `b` is legitimate.
    let sentinel = sentinel_var(&name);
    if std::env::var_os(&sentinel).is_some() {
        fail(
            EXIT_NOT_FOUND,
            format_args!(
                "refusing to re-enter myself for `{name}` ({sentinel} is already set); \
                 the real `{name}` is probably not on PATH"
            ),
        );
    }

    let target = match find_target(&name, &own_path) {
        Some(target) => target,
        None => fail(
            EXIT_NOT_FOUND,
            format_args!("no `{name}` found on PATH besides myself"),
        ),
    };
    trace!("target: {}", target.display());

    let env_file = config_dir().join(format!("{name}.env"));
    let pairs = load_env_file(&env_file);

    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    trace!("args: {args:?}");

    let mut command = Command::new(&target);
    command.args(&args);
    for (key, value) in &pairs {
        command.env(key, value);
    }
    command.env(&sentinel, "1");

    run(command, &target)
}

/// The stem of a path, e.g. `C:\tools\node.exe` -> `node`. Case is preserved: env file names are
/// matched literally so the same file works on a case-sensitive filesystem.
fn name_of(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    (!stem.is_empty()).then(|| stem.to_string())
}

fn sentinel_var(name: &str) -> String {
    let suffix: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("RWE_ACTIVE_{suffix}")
}

/// `$RWE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/rwe`, else `~/.config/rwe`. The same layout on every
/// platform, so there is only ever one place to look.
fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RWE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("rwe");
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".config").join("rwe")
}

/// Missing env file is the normal case and stays silent. An env file that exists but cannot be
/// read is fatal: running the target without the variables it was installed to inject would
/// silently do the wrong thing.
fn load_env_file(path: &Path) -> Vec<(String, String)> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            trace!("no env file at {}", path.display());
            return Vec::new();
        }
        Err(err) => fail(
            EXIT_CONFIG,
            format_args!("cannot read {}: {err}", path.display()),
        ),
    };

    let (pairs, warnings) = parse_env_file(&text);
    for warning in warnings {
        eprintln!("rwe: {}: {warning}", path.display());
    }
    if debug_enabled() {
        // Values are deliberately not printed: env files routinely hold tokens.
        let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        trace!("{}: injecting {}", path.display(), keys.join(", "));
    }
    pairs
}

/// One `name=value` per line. Blank lines and `#` comments are skipped, the split is on the first
/// `=`, and a matched pair of surrounding double quotes is stripped from the value. No variable
/// expansion and no escape sequences: either would mean implementing a shell.
fn parse_env_file(text: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut pairs = Vec::new();
    let mut warnings = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            warnings.push(format!("line {}: no `=`, skipped", index + 1));
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            warnings.push(format!("line {}: empty name, skipped", index + 1));
            continue;
        }
        pairs.push((key.to_string(), unquote(value).to_string()));
    }

    (pairs, warnings)
}

fn unquote(value: &str) -> &str {
    let value = value.trim_end();
    match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        Some(inner) if value.len() >= 2 => inner,
        _ => value,
    }
}

/// The first entry on `PATH` that matches the name, exists, and is not this binary. The current
/// directory is never searched, unlike `cmd.exe`: a `node.exe` dropped into a project folder must
/// not be able to take over.
fn find_target(name: &str, own_path: &Path) -> Option<PathBuf> {
    let own_real = own_path.canonicalize().ok();
    let path_var = std::env::var_os("PATH")?;

    for candidate in candidates(name, &path_var, &extensions()) {
        if !candidate.is_file() {
            continue;
        }
        let real = candidate.canonicalize().ok();
        if real.is_some() && real == own_real {
            trace!("skipping myself at {}", candidate.display());
            continue;
        }
        return Some(candidate);
    }
    None
}

/// `PATHEXT` on Windows, so a `.cmd` or `.bat` target resolves the way the shell would. An empty
/// extension comes first on Unix and last on Windows, matching each platform's own lookup order.
fn extensions() -> Vec<String> {
    if !cfg!(windows) {
        return vec![String::new()];
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut exts: Vec<String> = pathext
        .split(';')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_string)
        .collect();
    exts.push(String::new());
    exts
}

fn candidates(name: &str, path_var: &OsString, extensions: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in std::env::split_paths(path_var).filter(|d| !d.as_os_str().is_empty()) {
        for ext in extensions {
            out.push(dir.join(format!("{name}{ext}")));
        }
    }
    out
}

#[cfg(unix)]
fn run(mut command: Command, target: &Path) -> ! {
    use std::os::unix::process::CommandExt;
    // `exec` replaces us outright, so there is no exit code to propagate and no signal handling to
    // get right. It only returns on failure.
    let err = command.exec();
    fail(
        EXIT_SPAWN_FAILED,
        format_args!("cannot execute {}: {err}", target.display()),
    )
}

/// Windows has no `exec`, so we spawn and wait. See ADR-0001 for why we never exit early.
#[cfg(windows)]
fn run(mut command: Command, target: &Path) -> ! {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    // Console control events reach every process in the group, so the target gets its own Ctrl-C.
    // We must only stop dying on ours, or the shell would return a prompt mid-shutdown.
    unsafe { SetConsoleCtrlHandler(None, 1) };

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => fail(
            EXIT_SPAWN_FAILED,
            format_args!("cannot start {}: {err}", target.display()),
        ),
    };

    // Held until we exit: closing the job kills anything still inside it, so killing rwe cannot
    // leave the target orphaned.
    let _job = unsafe { attach_to_job(child.as_raw_handle() as HANDLE) };

    let status = match child.wait() {
        Ok(status) => status,
        Err(err) => fail(
            EXIT_SPAWN_FAILED,
            format_args!("cannot wait for {}: {err}", target.display()),
        ),
    };
    std::process::exit(status.code().unwrap_or(EXIT_SPAWN_FAILED));

    /// Returns the job handle, which must outlive the target; `None` if the job could not be set
    /// up, which is not worth failing the launch over.
    unsafe fn attach_to_job(process: HANDLE) -> Option<OwnedJob> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return None;
        }
        let job = OwnedJob(job);

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return None;
        }
        if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
            return None;
        }
        Some(job)
    }
}

#[cfg(windows)]
struct OwnedJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for OwnedJob {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs_and_ignores_noise() {
        let (pairs, warnings) = parse_env_file(
            "# comment\n\nFOO=bar\n  SPACED  =  padded  \nQUOTED=\"a b\"\nURL=http://x/?a=1\n",
        );
        assert_eq!(
            pairs,
            vec![
                ("FOO".into(), "bar".into()),
                ("SPACED".into(), "  padded".into()),
                ("QUOTED".into(), "a b".into()),
                ("URL".into(), "http://x/?a=1".into()),
            ]
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn warns_but_keeps_going_on_bad_lines() {
        let (pairs, warnings) = parse_env_file("GOOD=1\nnonsense\n=orphan\nALSO_GOOD=2\n");
        assert_eq!(
            pairs,
            vec![
                ("GOOD".into(), "1".into()),
                ("ALSO_GOOD".into(), "2".into())
            ]
        );
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn empty_value_is_an_empty_string_not_an_unset() {
        let (pairs, _) = parse_env_file("FOO=\n");
        assert_eq!(pairs, vec![("FOO".into(), String::new())]);
    }

    #[test]
    fn later_duplicate_wins_by_application_order() {
        let (pairs, _) = parse_env_file("FOO=first\nFOO=second\n");
        assert_eq!(pairs.last().unwrap().1, "second");
    }

    #[test]
    fn unquoting_needs_both_quotes() {
        assert_eq!(unquote("\"a b\""), "a b");
        assert_eq!(unquote("\"unbalanced"), "\"unbalanced");
        assert_eq!(unquote("\""), "\"");
        assert_eq!(unquote("\"\""), "");
    }

    #[test]
    fn name_comes_from_the_stem() {
        assert_eq!(name_of(Path::new("/usr/bin/node")).as_deref(), Some("node"));
        assert_eq!(
            name_of(Path::new(r"C:\tools\Node.exe")).as_deref(),
            Some("Node"),
            "case is preserved so the env file name is matched literally"
        );
    }

    #[test]
    fn sentinel_is_per_name() {
        assert_eq!(sentinel_var("node"), "RWE_ACTIVE_NODE");
        assert_ne!(sentinel_var("node"), sentinel_var("deno"));
        assert_eq!(sentinel_var("my-tool.v2"), "RWE_ACTIVE_MY_TOOL_V2");
    }

    #[test]
    fn candidates_cover_every_dir_and_extension_in_order() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let path: OsString = format!("{}{sep}{}", dir("a"), dir("b")).into();
        let exts = vec![".EXE".to_string(), String::new()];

        let found = candidates("node", &path, &exts);

        assert_eq!(
            found,
            vec![
                PathBuf::from(dir("a")).join("node.EXE"),
                PathBuf::from(dir("a")).join("node"),
                PathBuf::from(dir("b")).join("node.EXE"),
                PathBuf::from(dir("b")).join("node"),
            ]
        );
    }

    #[test]
    fn candidates_never_include_the_current_directory() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        // An empty PATH entry means "current directory" to the shell. It must not mean that here.
        let path: OsString = format!("{sep}{}{sep}", dir("a")).into();

        let found = candidates("node", &path, &[String::new()]);

        assert_eq!(found, vec![PathBuf::from(dir("a")).join("node")]);
    }

    fn dir(name: &str) -> String {
        if cfg!(windows) {
            format!(r"C:\{name}")
        } else {
            format!("/{name}")
        }
    }
}
