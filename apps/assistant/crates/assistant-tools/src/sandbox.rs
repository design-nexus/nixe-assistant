//! The bubblewrap sandbox for terminal commands the assistant writes itself.
//!
//! Inside: the system read-only, an empty home with only the working folder writable and
//! the configured toolchain folders readable, a private /tmp, and no `XDG_RUNTIME_DIR`, so
//! no Wayland, D-Bus or assistant socket. Network is shared only when configured. A
//! repository's hooks and config stay read-only: git runs them later, outside the sandbox.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Sandbox {
    pub network: bool,
    /// Folders under HOME the command may read (already expanded).
    pub read_only: Vec<PathBuf>,
}

impl Sandbox {
    pub fn from_config(cfg: &assistant_core::config::TerminalCfg, home: &Path) -> Self {
        let read_only =
            cfg.sandbox_read_only.iter().filter_map(|p| crate::fs::guard(home, p, crate::fs::Access::Read).ok()).collect();
        Self { network: cfg.sandbox_network, read_only }
    }
}

/// Most entries looked at under `cwd` for repositories; a bigger folder isn't shared.
const MAX_WALK: usize = 50_000;

/// What inside `cwd` runs later outside the sandbox and so stays read-only: the folders of
/// `fs::code_dirs`, and the hooks and config of every repository below `cwd` (submodules'
/// included). `Err` when `cwd` is too big to check.
fn protected_inside(home: &Path, cwd: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> =
        crate::fs::code_dirs(home).into_iter().filter(|d| d.starts_with(cwd) && d != cwd && d.exists()).collect();
    let mut seen = 0usize;
    let mut stack = vec![(cwd.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        // A git folder: HEAD and objects (`.git` itself, a bare one, or one in .git/modules).
        if dir.join("HEAD").is_file() && dir.join("objects").is_dir() {
            out.push(dir.join("hooks"));
            out.push(dir.join("config"));
        }
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            seen += 1;
            anyhow::ensure!(
                seen <= MAX_WALK,
                "{} holds too many files to check for repositories; use a project folder",
                cwd.display()
            );
            let name = entry.file_name();
            let skip = matches!(name.to_str(), Some("objects" | "node_modules" | "target"));
            if !skip && depth < 12 && entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    Ok(out)
}

/// The bwrap arguments (before `--` and the command) for running in `cwd`.
pub fn bwrap_args(sandbox: &Sandbox, home: &Path, cwd: &Path) -> anyhow::Result<Vec<String>> {
    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    push(&["--die-with-parent", "--new-session", "--unshare-all"]);
    if sandbox.network {
        push(&["--share-net"]);
    }
    push(&["--ro-bind", "/usr", "/usr", "--ro-bind", "/etc", "/etc"]);
    for dir in ["/bin", "/lib", "/lib64", "/sbin"] {
        match std::fs::read_link(dir) {
            Ok(target) => push(&["--symlink", &target.display().to_string(), dir]),
            Err(_) if Path::new(dir).exists() => push(&["--ro-bind", dir, dir]),
            Err(_) => {}
        }
    }
    push(&["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
    if sandbox.network {
        // /etc/resolv.conf usually points into systemd-resolved's runtime folder.
        push(&["--ro-bind-try", "/run/systemd/resolve", "/run/systemd/resolve"]);
    }
    let home_s = home.display().to_string();
    push(&["--tmpfs", &home_s]);
    for dir in &sandbox.read_only {
        if dir.starts_with(home) && !cwd.starts_with(dir) {
            let d = dir.display().to_string();
            push(&["--ro-bind-try", &d, &d]);
        }
    }
    let cwd_s = cwd.display().to_string();
    push(&["--bind", &cwd_s, &cwd_s]);
    if let Some(git) = repository(home, cwd) {
        for part in ["hooks", "config"] {
            let p = git.join(part).display().to_string();
            push(&["--ro-bind-try", &p, &p]);
        }
    }
    for p in protected_inside(home, cwd)? {
        let p = p.display().to_string();
        push(&["--ro-bind-try", &p, &p]);
    }
    // Secrets inside a readable folder or the working one (~/.cargo/credentials.toml,
    // ~/.config/gh) stay hidden.
    for secret in crate::fs::SECRET.iter().map(|s| home.join(s)) {
        if sandbox.read_only.iter().chain(std::iter::once(&cwd.to_path_buf())).any(|d| secret.starts_with(d)) {
            let p = secret.display().to_string();
            if secret.is_dir() {
                push(&["--tmpfs", &p]);
            } else if secret.exists() {
                push(&["--ro-bind", "/dev/null", &p]);
            }
        }
    }
    push(&["--chdir", &cwd_s, "--clearenv"]);
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin".into());
    push(&["--setenv", "PATH", &path, "--setenv", "HOME", &home_s, "--setenv", "TERM", "dumb"]);
    if let Ok(lang) = std::env::var("LANG") {
        push(&["--setenv", "LANG", &lang]);
    }
    Ok(a)
}

/// The `.git` folder of the repository `cwd` is in, looking up to HOME. Its hooks and config
/// run code when Ken next uses git, so they are bound read-only over the writable `cwd`.
fn repository(home: &Path, cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors().take_while(|d| d.starts_with(home) && *d != home).map(|d| d.join(".git")).find(|g| g.is_dir())
}

/// The full command line: `bwrap <args> -- <argv>`.
pub fn wrap(sandbox: &Sandbox, home: &Path, cwd: &Path, argv: Vec<String>) -> anyhow::Result<Vec<String>> {
    let mut out = vec!["bwrap".to_string()];
    out.extend(bwrap_args(sandbox, home, cwd)?);
    out.push("--".into());
    out.extend(argv);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn arguments() {
        let home = Path::new("/home/u");
        let s = Sandbox { network: false, read_only: vec![home.join(".cargo")] };
        let a = bwrap_args(&s, home, &home.join("proj")).unwrap();
        assert!(a.contains(&"--unshare-all".to_string()) && !a.contains(&"--share-net".to_string()));
        assert!(pair(&a, "--tmpfs", "/home/u"));
        assert!(pair(&a, "--bind", "/home/u/proj") && pair(&a, "--chdir", "/home/u/proj"));
        assert!(pair(&a, "--ro-bind-try", "/home/u/.cargo"));
        assert!(a.contains(&"--clearenv".to_string()));
        assert!(!a.iter().any(|x| x.contains("XDG_RUNTIME_DIR")));
        let net = bwrap_args(&Sandbox { network: true, read_only: vec![] }, home, &home.join("proj")).unwrap();
        assert!(net.contains(&"--share-net".to_string()) && pair(&net, "--ro-bind-try", "/run/systemd/resolve"));
        // The home tmpfs comes before the binds that punch through it.
        let tmpfs = a.iter().position(|x| x == "/home/u").unwrap();
        let bind = a.iter().position(|x| x == "--bind").unwrap();
        assert!(tmpfs < bind);
    }

    /// Runs the real bubblewrap when it's installed.
    #[test]
    fn contains_a_real_command() {
        if std::process::Command::new("bwrap").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id"), "KEY").unwrap();
        std::fs::create_dir_all(home.join("proj")).unwrap();
        let s = Sandbox { network: false, read_only: vec![] };
        let run = |script: &str| {
            let argv = wrap(&s, &home, &home.join("proj"), vec!["sh".into(), "-c".into(), script.into()]).unwrap();
            std::process::Command::new(&argv[0]).args(&argv[1..]).output().unwrap()
        };
        assert!(!run("cat ~/.ssh/id").status.success(), "secrets are hidden");
        assert!(!run("echo x > ~/outside").status.success() || !home.join("outside").exists(), "writes outside cwd don't land");
        assert!(run("echo x > inside").status.success());
        assert_eq!(std::fs::read_to_string(home.join("proj/inside")).unwrap(), "x\n");
        let env = run("echo \"${XDG_RUNTIME_DIR:-unset}\"");
        assert_eq!(String::from_utf8_lossy(&env.stdout).trim(), "unset");
        assert!(!run("ls /run/user").status.success() || run("ls -A /run/user").stdout.is_empty(), "no runtime sockets");
        let sock = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default() + "/nixe-assistant/assistant.sock";
        assert!(!run(&format!("test -e '{sock}'")).status.success(), "the assistant socket isn't there");
        assert!(!run("test -e /run/dbus/system_bus_socket").status.success(), "no system bus");
        // No network unless configured.
        assert!(run("cat /proc/net/dev").status.success());
        assert!(!run("tail -n +3 /proc/net/dev | grep -qv ' lo:'").status.success(), "only loopback");
    }

    /// A generated command can't plant a git hook or config that runs later, outside.
    #[test]
    fn repository_hooks_stay_read_only() {
        if std::process::Command::new("bwrap").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(home.join("repo/.git/hooks")).unwrap();
        std::fs::create_dir_all(home.join("repo/src")).unwrap();
        std::fs::write(home.join("repo/.git/config"), "[core]\n").unwrap();
        let s = Sandbox { network: false, read_only: vec![] };
        for cwd in [home.join("repo"), home.join("repo/src")] {
            let run = |script: &str| {
                let argv = wrap(&s, &home, &cwd, vec!["sh".into(), "-c".into(), script.into()]).unwrap();
                std::process::Command::new(&argv[0]).args(&argv[1..]).output().unwrap()
            };
            let git = if cwd.ends_with("src") { "../.git" } else { ".git" };
            assert!(!run(&format!("echo evil > {git}/hooks/pre-commit")).status.success(), "{cwd:?}");
            assert!(!run(&format!("echo '[core] fsmonitor = evil' >> {git}/config")).status.success(), "{cwd:?}");
            assert!(run("echo ok > file").status.success(), "the working folder stays writable");
        }
        assert!(!home.join("repo/.git/hooks/pre-commit").exists());
        assert_eq!(std::fs::read_to_string(home.join("repo/.git/config")).unwrap(), "[core]\n");
    }

    /// A folder holding repositories, or holding config that runs later, shares only the
    /// rest: their hooks and those folders stay read-only. A code folder itself isn't shared.
    #[test]
    fn nested_repositories_and_code_folders_stay_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let git = home.join("projects/app/.git");
        std::fs::create_dir_all(git.join("hooks")).unwrap();
        std::fs::create_dir_all(git.join("objects")).unwrap();
        std::fs::create_dir_all(git.join("modules/lib/hooks")).unwrap();
        std::fs::create_dir_all(git.join("modules/lib/objects")).unwrap();
        std::fs::write(git.join("modules/lib/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("config"), "[core]\n").unwrap();
        std::fs::create_dir_all(home.join(".config/hypr")).unwrap();
        let inside = protected_inside(&home, &home.join("projects")).unwrap();
        for p in [git.join("hooks"), git.join("config"), git.join("modules/lib/hooks")] {
            assert!(inside.contains(&p), "{p:?} in {inside:?}");
        }
        assert!(protected_inside(&home, &home.join(".config")).unwrap().contains(&home.join(".config/hypr")));
        assert!(!crate::fs::sandbox_dir_ok(&home, &home.join(".config/hypr")), "a code folder isn't a working folder");
        assert!(!crate::fs::sandbox_dir_ok(&home, &git.join("hooks")));
        assert!(crate::fs::sandbox_dir_ok(&home, &home.join("projects")));
        if std::process::Command::new("bwrap").arg("--version").output().is_err() {
            return;
        }
        let s = Sandbox { network: false, read_only: vec![] };
        let run = |cwd: &Path, script: &str| {
            let argv = wrap(&s, &home, cwd, vec!["sh".into(), "-c".into(), script.into()]).unwrap();
            std::process::Command::new(&argv[0]).args(&argv[1..]).output().unwrap()
        };
        assert!(!run(&home.join("projects"), "echo evil > app/.git/hooks/pre-commit").status.success());
        assert!(!run(&home.join("projects"), "echo evil > app/.git/modules/lib/hooks/post-checkout").status.success());
        assert!(!run(&home.join(".config"), "echo 'exec = evil' > hypr/hyprland.conf").status.success());
        assert!(run(&home.join("projects"), "echo ok > app/notes.txt").status.success());
    }
}
