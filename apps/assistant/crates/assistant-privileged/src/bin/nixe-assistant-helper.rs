//! Runs as root through pkexec for Nixe Assistant's admin tools. It does one checked
//! action per run and nothing else; every argument is parsed again here, because the
//! caller is an ordinary user's program.
//!
//!   nixe-assistant-helper install PKG|REPO/PKG…    configured repositories only, -S --needed
//!   nixe-assistant-helper remove PKG…              -Rs, refused if it would remove a protected package
//!   nixe-assistant-helper update                   -Syu
//!   nixe-assistant-helper service start|stop|restart UNIT.service
//!   nixe-assistant-helper --version                needs no root; the assistant compares it
//!
//! Prints one JSON object on success; on failure, a message on stderr and exit 1. Every
//! request is appended to /var/log/nixe-assistant/privileged.log.

use assistant_privileged::{check_repo, protected_package, Action, Deny, ADMIN_CONFIG, HELPER_VERSION, LOG};
use serde_json::{json, Value};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

const PACMAN: &str = "/usr/bin/pacman";
const PACMAN_CONF: &str = "/usr/bin/pacman-conf";
const SYSTEMCTL: &str = "/usr/bin/systemctl";
/// The only programs that may ask: the first-party plugin worker runs the admin tools (the
/// daemon itself does when plugins are off).
const CALLERS: &[&str] = &["nixe-assistant-plugin-worker", "nixe-assistantd"];
/// Keeps the reply (and the daemon's context) small after a long update.
const OUTPUT_TAIL: usize = 4000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if refs == ["--version"] {
        println!("{HELPER_VERSION}");
        return;
    }
    let started = Instant::now();
    let invoker = match caller() {
        Ok(uid) => uid,
        Err(e) => {
            // Not run by pkexec for the assistant: nothing to log as root, and nothing done.
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let result = run(&refs);
    log(invoker, &refs, &result, started);
    match result {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Root, started by pkexec (which records the real user), on behalf of the assistant daemon.
/// The parent check narrows who can ask; the password polkit asked for is what authorizes.
fn caller() -> Result<u32> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("nixe-assistant-helper must be run by the assistant through pkexec".into());
    }
    let uid: u32 = std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or("nixe-assistant-helper must be run by the assistant through pkexec")?;
    let parent = std::os::unix::process::parent_id();
    let exe = std::fs::read_link(format!("/proc/{parent}/exe")).map_err(|e| format!("can't identify the caller: {e}"))?;
    let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // A binary replaced while running reads back as "name (deleted)".
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    let owner = std::fs::metadata(format!("/proc/{parent}")).map(|m| m.uid()).ok();
    if !CALLERS.contains(&name) || owner != Some(uid) {
        return Err(format!("only the assistant daemon may use this helper, not {}", exe.display()));
    }
    Ok(uid)
}

fn run(args: &[&str]) -> Result<Value> {
    let action = Action::parse(args)?;
    deny_list()?.check(&action)?;
    match &action {
        Action::Install { packages } => {
            let repos = pacman_repos()?;
            for p in packages {
                check_repo(p, &repos)?;
            }
            unlocked()?;
            let out = exec(PACMAN, &[&["-S", "--needed", "--noconfirm", "--noprogressbar", "--"], &strs(packages)[..]].concat())?;
            Ok(json!({"action": action.to_string(), "output": tail(&out)}))
        }
        Action::Remove { packages } => {
            unlocked()?;
            let list = exec(PACMAN, &[&["-Rsp", "--print-format", "%n", "--"], &strs(packages)[..]].concat())?;
            let removing: Vec<&str> = list.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            if let Some(p) = removing.iter().find(|p| protected_package(p)) {
                return Err(format!("removing that would take {p} with it, which the assistant never removes"));
            }
            let out = exec(PACMAN, &[&["-Rs", "--noconfirm", "--noprogressbar", "--"], &strs(packages)[..]].concat())?;
            Ok(json!({"action": action.to_string(), "removed": removing, "output": tail(&out)}))
        }
        Action::Update => {
            unlocked()?;
            let out = exec(PACMAN, &["-Syu", "--noconfirm", "--noprogressbar"])?;
            Ok(json!({"action": action.to_string(), "output": tail(&out)}))
        }
        Action::Service { action: verb, unit } => {
            exec(SYSTEMCTL, &[verb.as_str(), "--", unit])?;
            let state = exec(SYSTEMCTL, &["show", "-p", "ActiveState", "--value", "--", unit])?;
            Ok(json!({"action": action.to_string(), "active_state": state.trim()}))
        }
    }
}

fn strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

/// The extra deny lists, honoured only from a file nobody but root can change.
fn deny_list() -> Result<Deny> {
    let path = Path::new(ADMIN_CONFIG);
    let Ok(meta) = std::fs::metadata(path) else { return Ok(Deny::default()) };
    if meta.uid() != 0 || meta.permissions().mode() & 0o022 != 0 {
        return Err(format!("{ADMIN_CONFIG} must be owned by root and not writable by others"));
    }
    Deny::parse(&std::fs::read_to_string(path).map_err(|e| format!("{ADMIN_CONFIG}: {e}"))?)
}

fn pacman_repos() -> Result<Vec<String>> {
    Ok(exec(PACMAN_CONF, &["--repo-list"])?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

fn unlocked() -> Result<()> {
    if Path::new("/var/lib/pacman/db.lck").exists() {
        return Err("another package operation is running (pacman's database is locked)".into());
    }
    Ok(())
}

/// Runs with a fixed environment: pkexec already cleared most of it, and nothing the
/// caller set reaches pacman or its hooks.
fn exec(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/bin")
        .env("HOME", "/root")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&output.stderr);
    let out = String::from_utf8_lossy(&output.stdout);
    let text = if err.trim().is_empty() { out.trim() } else { err.trim() };
    Err(if text.is_empty() { format!("{program} failed") } else { tail(text) })
}

fn tail(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().rev().nth(OUTPUT_TAIL) {
        Some((i, _)) => format!("…{}", &text[i..]),
        None => text.to_string(),
    }
}

/// One JSON line per request. Readable by everyone: it holds package and unit names, no secrets.
fn log(uid: u32, args: &[&str], result: &Result<Value>, started: Instant) {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let line = json!({
        "at_ms": at,
        "uid": uid,
        "args": args,
        "ok": result.is_ok(),
        "error": result.as_ref().err(),
        "duration_ms": started.elapsed().as_millis() as u64,
    });
    let path = Path::new(LOG);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).mode(0o644).open(path);
    if let Ok(mut file) = file {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end() {
        let long = "x".repeat(OUTPUT_TAIL + 50) + "end";
        let t = tail(&long);
        assert!(t.starts_with('…') && t.ends_with("end") && t.chars().count() == OUTPUT_TAIL + 2);
        assert_eq!(tail("  short \n"), "short");
    }

    #[test]
    fn refuses_without_pkexec() {
        // Tests don't run as root, so this is the "run by hand" path.
        if unsafe { libc::geteuid() } != 0 {
            assert!(caller().is_err());
        }
    }
}
