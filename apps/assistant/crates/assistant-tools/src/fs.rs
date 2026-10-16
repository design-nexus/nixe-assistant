//! File tools inside HOME: one path guard, and writes that keep a backup to undo from.
//!
//! Every path is resolved (symlinks included) before it is checked. Secret folders are
//! off limits for reading and writing; the assistant's own config, state and data are
//! read-only, so a tool can't edit the policy that governs it. Overwrites, appends, moves
//! over a file and trashing keep a copy in `file-backups/<id>` that `files.undo` restores.

use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// Never read or written: keys, keyrings, password stores, browser profiles.
const SECRET: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".password-store",
    ".local/share/keyrings",
    ".pki",
    ".mozilla",
    ".config/chromium",
    ".config/google-chrome",
    ".config/BraveSoftware",
];
/// Read-only to tools: the assistant's own configuration, journal and data.
const SELF: &[&str] = &[".config/nixe-assistant", ".local/state/nixe-assistant", ".local/share/nixe-assistant"];
/// Run as the user later, outside any sandbox: shell startup, `~/.local/bin`, session and
/// autostart services, launchers, Hyprland and Omarchy hooks, git configuration. Writing
/// them needs a key press each time.
const RUNS_CODE: &[&str] = &[
    ".local/bin",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".profile",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".xprofile",
    ".config/fish",
    ".config/systemd",
    ".config/autostart",
    ".config/environment.d",
    ".config/uwsm",
    ".config/hypr",
    ".config/omarchy/hooks",
    ".local/share/applications",
    ".gitconfig",
    ".config/git",
];
pub const MAX_WRITE: usize = 1024 * 1024;
const MAX_LIST: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// Canonical HOME.
pub fn home() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME unavailable")?;
    Ok(Path::new(&home).canonicalize()?)
}

fn under(path: &Path, home: &Path, list: &[&str]) -> bool {
    list.iter().any(|p| path.starts_with(home.join(p)))
}

/// Resolve `input` (`~/…`, absolute, or relative to HOME) and check it. For writes the file
/// may not exist yet; its parent must.
pub fn guard(home: &Path, input: &str, access: Access) -> Result<PathBuf> {
    ensure!(!input.is_empty() && !input.contains('\0'), "invalid path");
    let raw = if input == "~" {
        home.to_path_buf()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        home.join(input)
    };
    let path = match raw.canonicalize() {
        Ok(p) => p,
        Err(_) if access == Access::Write => {
            let name = raw.file_name().context("path has no file name")?;
            ensure!(Path::new(name).components().all(|c| matches!(c, Component::Normal(_))), "invalid file name");
            let parent = raw.parent().context("path has no parent")?;
            parent.canonicalize().context("parent folder doesn't exist")?.join(name)
        }
        Err(e) => return Err(e).with_context(|| format!("{input} not found")),
    };
    ensure!(path.starts_with(home), "path is outside the home directory");
    ensure!(!under(&path, home, SECRET), "that folder holds secrets and is off limits");
    if access == Access::Write {
        ensure!(path != home, "won't write the home directory itself");
        ensure!(!under(&path, home, SELF) && !in_own_dirs(&path), "the assistant's own files are read-only to tools");
    }
    Ok(path)
}

/// Whether writing `path` (already through `guard`) changes something that runs as the user
/// later, outside the sandbox. Includes any repository's hooks and config.
pub fn runs_code(home: &Path, path: &Path) -> bool {
    if under(path, home, RUNS_CODE) {
        return true;
    }
    let parts: Vec<_> = path.components().collect();
    parts.windows(2).any(|w| {
        w[0].as_os_str() == ".git" && (w[1].as_os_str() == "hooks" || w[1].as_os_str() == "config")
    })
}

/// The files a `files.undo` would write, for the same check.
pub fn undo_targets(home: &Path, id: &str) -> Vec<PathBuf> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(backups(home).join(id).join("backup.json")) else { return Vec::new() };
    let manifest: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    manifest["entries"].as_array().into_iter().flatten().filter_map(|e| e["original"].as_str()).map(PathBuf::from).collect()
}

/// Open a file without following a symlink at its last component: one swapped in after
/// `guard` checked the path fails here instead of being read through.
pub fn open_nofollow(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("can't open {} (a link?)", path.display()))
}

fn read_nofollow(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    open_nofollow(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The assistant's folders where XDG variables put them, besides the defaults in `SELF`.
fn in_own_dirs(path: &Path) -> bool {
    use assistant_core::paths;
    [paths::config_dir(), paths::state_dir(), paths::data_dir()]
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| path.starts_with(d))
}

/// Whether `dir` may be a sandbox's writable working folder.
pub fn sandbox_dir_ok(home: &Path, dir: &Path) -> bool {
    dir.starts_with(home) && dir != home && !under(dir, home, SECRET) && !under(dir, home, SELF) && !in_own_dirs(dir)
}

fn backups(home: &Path) -> PathBuf {
    home.join(".local/state/nixe-assistant/file-backups")
}

fn new_backup_id() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{ms}-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Record the files a change will touch: copies of those that exist, a note of those that
/// don't (so undo removes what the change created). Returns the backup ID.
fn back_up(home: &Path, paths: &[&Path]) -> Result<String> {
    let id = new_backup_id();
    let dir = backups(home).join(&id);
    std::fs::create_dir_all(&dir)?;
    let mut entries = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let existed = path.is_file();
        if existed {
            std::fs::write(dir.join(i.to_string()), read_nofollow(path)?)?;
        }
        entries.push(json!({"original": path, "stored": i.to_string(), "existed": existed}));
    }
    std::fs::write(dir.join("backup.json"), serde_json::to_vec_pretty(&json!({"id": id, "entries": entries}))?)?;
    Ok(id)
}

fn undo_record(backup: &str) -> Value {
    json!({"version": 1, "tool": "files.undo", "arguments": {"backup": backup}})
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("no parent folder")?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(bytes)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(tmp.path(), meta.permissions());
    }
    tmp.persist(path)?;
    Ok(())
}

pub fn list(home: &Path, input: &str) -> Result<Value> {
    let dir = guard(home, input, Access::Read)?;
    ensure!(dir.is_dir(), "not a folder");
    let mut entries = Vec::new();
    let mut total = 0;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        total += 1;
        if entries.len() >= MAX_LIST {
            continue;
        }
        let meta = entry.metadata()?;
        let kind = if meta.is_dir() {
            "dir"
        } else if meta.is_symlink() {
            "link"
        } else {
            "file"
        };
        entries.push(json!({"name": entry.file_name().to_string_lossy(), "kind": kind, "size": meta.len()}));
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({"path": dir, "entries": entries, "total": total, "truncated": total > MAX_LIST}))
}

/// Files under `input` whose name contains `name` (case-insensitive), skipping hidden and
/// guarded folders; bounded in depth and count.
pub fn find_names(home: &Path, input: &str, name: &str) -> Result<Value> {
    let root = guard(home, input, Access::Read)?;
    ensure!(root.is_dir(), "not a folder");
    let needle = name.to_lowercase();
    let mut found = Vec::new();
    let mut visited = 0;
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            visited += 1;
            if visited > 20_000 || found.len() >= 50 {
                return Ok(json!({"path": root, "matches": found, "truncated": true}));
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if file_name.to_lowercase().contains(&needle) {
                found.push(path.display().to_string());
            }
            if is_dir && depth < 6 && !under(&path, home, SECRET) {
                stack.push((path, depth + 1));
            }
        }
    }
    Ok(json!({"path": root, "matches": found, "truncated": false}))
}

pub fn write(home: &Path, input: &str, content: &str, mode: &str) -> Result<Value> {
    ensure!(content.len() <= MAX_WRITE, "content is over 1 MiB");
    let path = guard(home, input, Access::Write)?;
    ensure!(!path.is_dir(), "that's a folder");
    let exists = path.exists();
    match mode {
        "create" => ensure!(!exists, "file already exists; use overwrite or append"),
        "overwrite" | "append" => {}
        _ => bail!("mode must be create, overwrite or append"),
    }
    let backup = back_up(home, &[&path])?;
    let bytes = if mode == "append" && exists {
        let mut old = read_nofollow(&path)?;
        old.extend_from_slice(content.as_bytes());
        old
    } else {
        content.as_bytes().to_vec()
    };
    atomic_write(&path, &bytes)?;
    let verified = std::fs::read(&path).is_ok_and(|now| now == bytes);
    Ok(
        json!({"path": path, "bytes": bytes.len(), "created": !exists, "backup": backup, "verified": verified, "undo": undo_record(&backup)}),
    )
}

pub fn rename(home: &Path, from: &str, to: &str, overwrite: bool) -> Result<Value> {
    let source = guard(home, from, Access::Write)?;
    ensure!(source.exists(), "{from} not found");
    let mut target = guard(home, to, Access::Write)?;
    if target.is_dir() {
        target = target.join(source.file_name().context("source has no name")?);
        target = guard(home, &target.display().to_string(), Access::Write)?;
    }
    ensure!(target != source, "source and target are the same");
    ensure!(!target.exists() || overwrite, "target exists; pass overwrite to replace it");
    ensure!(!target.is_dir(), "won't replace a folder");
    // Undo moves the file back: the backup records both ends.
    let backup = if source.is_file() { Some(back_up(home, &[&source, &target])?) } else { None };
    std::fs::rename(&source, &target).or_else(|_| -> Result<()> {
        ensure!(source.is_file(), "can't move a folder across file systems");
        std::fs::copy(&source, &target)?;
        std::fs::remove_file(&source)?;
        Ok(())
    })?;
    let verified = target.exists() && !source.exists();
    Ok(json!({"from": source, "to": target, "backup": backup, "verified": verified, "undo": backup.as_deref().map(undo_record)}))
}

pub fn trash(home: &Path, input: &str) -> Result<Value> {
    let path = guard(home, input, Access::Write)?;
    ensure!(path.exists(), "{input} not found");
    let backup =
        if path.is_file() && path.metadata()?.len() <= 50 * MAX_WRITE as u64 { Some(back_up(home, &[&path])?) } else { None };
    let out = std::process::Command::new("gio").arg("trash").arg(&path).output().context("gio trash")?;
    ensure!(out.status.success(), "trash failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    let verified = !path.exists();
    Ok(
        json!({"path": path, "backup": backup, "verified": verified, "undo": backup.as_deref().map(undo_record), "note": "Folders and large files go to the Trash without a backup."}),
    )
}

/// Put back what a backup recorded: restore copies, remove files the change created. The
/// current versions are backed up first, so an undo can itself be undone.
pub fn undo(home: &Path, id: &str) -> Result<Value> {
    ensure!(!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "invalid backup ID");
    let dir = backups(home).join(id);
    let manifest: Value = serde_json::from_slice(&std::fs::read(dir.join("backup.json")).context("no such backup")?)?;
    let entries = manifest["entries"].as_array().context("invalid backup")?;
    let originals: Vec<PathBuf> =
        entries.iter().map(|e| guard(home, e["original"].as_str().unwrap_or(""), Access::Write)).collect::<Result<_>>()?;
    let refs: Vec<&Path> = originals.iter().map(PathBuf::as_path).collect();
    let before = back_up(home, &refs)?;
    let mut verified = true;
    for (entry, path) in entries.iter().zip(&originals) {
        if entry["existed"] == true {
            let stored = dir.join(entry["stored"].as_str().unwrap_or(""));
            let bytes = std::fs::read(&stored)?;
            atomic_write(path, &bytes)?;
            verified &= std::fs::read(path).is_ok_and(|now| now == bytes);
        } else if path.is_file() {
            std::fs::remove_file(path)?;
            verified &= !path.exists();
        }
    }
    Ok(json!({"restored": id, "files": originals, "backup": before, "verified": verified}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        for d in [".ssh", ".config/nixe-assistant", "work"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join(".ssh/id_ed25519"), "KEY").unwrap();
        std::fs::write(home.join("work/a.txt"), "one").unwrap();
        (dir, home)
    }

    #[test]
    fn guard_refuses_secrets_self_and_escapes() {
        let (_d, h) = home();
        assert!(guard(&h, "~/work/a.txt", Access::Read).is_ok());
        assert!(guard(&h, "work/a.txt", Access::Read).is_ok(), "relative to HOME");
        assert!(guard(&h, "~/.ssh/id_ed25519", Access::Read).is_err());
        assert!(guard(&h, "~/work/../.ssh/id_ed25519", Access::Read).is_err());
        std::os::unix::fs::symlink(h.join(".ssh"), h.join("work/keys")).unwrap();
        assert!(guard(&h, "~/work/keys/id_ed25519", Access::Read).is_err(), "symlink into secrets");
        assert!(guard(&h, "/etc/hostname", Access::Read).is_err());
        assert!(guard(&h, "~/.config/nixe-assistant", Access::Read).is_ok(), "self is readable");
        assert!(guard(&h, "~/.config/nixe-assistant/config.toml", Access::Write).is_err(), "self is read-only");
        assert!(guard(&h, "~/work/new.txt", Access::Write).is_ok(), "new file in an existing folder");
        assert!(guard(&h, "~/missing/new.txt", Access::Write).is_err());
        assert!(guard(&h, "~", Access::Write).is_err());
        assert!(sandbox_dir_ok(&h, &h.join("work")));
        assert!(!sandbox_dir_ok(&h, &h) && !sandbox_dir_ok(&h, &h.join(".ssh")));
    }

    #[test]
    fn write_and_undo_round_trip() {
        let (_d, h) = home();
        let r = write(&h, "~/work/a.txt", "two", "overwrite").unwrap();
        assert_eq!((r["verified"].as_bool(), r["created"].as_bool()), (Some(true), Some(false)));
        assert_eq!(std::fs::read_to_string(h.join("work/a.txt")).unwrap(), "two");
        assert_eq!(r["undo"]["tool"], "files.undo");
        undo(&h, r["backup"].as_str().unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(h.join("work/a.txt")).unwrap(), "one");
        // Undo of a created file removes it.
        let r = write(&h, "~/work/b.txt", "new", "create").unwrap();
        assert!(write(&h, "~/work/b.txt", "again", "create").is_err());
        undo(&h, r["backup"].as_str().unwrap()).unwrap();
        assert!(!h.join("work/b.txt").exists());
        let r = write(&h, "~/work/a.txt", "+", "append").unwrap();
        assert_eq!(std::fs::read_to_string(h.join("work/a.txt")).unwrap(), "one+");
        assert!(r["verified"] == true);
        assert!(write(&h, "~/work/a.txt", &"x".repeat(MAX_WRITE + 1), "overwrite").is_err());
        assert!(undo(&h, "../../etc").is_err());
    }

    #[test]
    fn move_and_undo() {
        let (_d, h) = home();
        std::fs::write(h.join("work/c.txt"), "cee").unwrap();
        assert!(rename(&h, "~/work/c.txt", "~/work/a.txt", false).is_err(), "won't overwrite");
        let r = rename(&h, "~/work/c.txt", "~/work/d.txt", false).unwrap();
        assert!(r["verified"] == true && h.join("work/d.txt").exists() && !h.join("work/c.txt").exists());
        undo(&h, r["backup"].as_str().unwrap()).unwrap();
        assert!(h.join("work/c.txt").exists() && !h.join("work/d.txt").exists());
        let r = rename(&h, "~/work/c.txt", "~/work", false);
        assert!(r.is_err(), "moving into its own folder is the same path");
    }

    #[test]
    fn list_and_find() {
        let (_d, h) = home();
        let l = list(&h, "~/work").unwrap();
        assert_eq!(l["entries"][0]["name"], "a.txt");
        assert!(list(&h, "~/.ssh").is_err());
        let f = find_names(&h, "~", "A.TXT").unwrap();
        assert_eq!(f["matches"].as_array().unwrap().len(), 1);
        let f = find_names(&h, "~", "id_ed").unwrap();
        assert!(f["matches"].as_array().unwrap().is_empty(), "hidden secrets are skipped");
    }

    #[test]
    fn traversal_and_link_escapes() {
        let (_d, h) = home();
        let outside = tempfile::tempdir().unwrap();
        let out = outside.path().canonicalize().unwrap();
        std::fs::write(out.join("x"), "x").unwrap();
        for input in ["~/../x", "../x", "~/work/../../x", "/etc/passwd", "/proc/self/environ", "work/\0x", ""] {
            assert!(guard(&h, input, Access::Read).is_err(), "{input}");
        }
        assert!(guard(&h, &format!("~/work/{}", "../".repeat(20)), Access::Read).is_err());
        // Links out of HOME, into secrets and into the assistant's own files.
        std::os::unix::fs::symlink(&out, h.join("work/out")).unwrap();
        assert!(guard(&h, "~/work/out/x", Access::Read).is_err());
        assert!(guard(&h, "~/work/out/new", Access::Write).is_err(), "new file under a linked folder");
        std::os::unix::fs::symlink(h.join(".ssh/id_ed25519"), h.join("work/key")).unwrap();
        assert!(guard(&h, "~/work/key", Access::Read).is_err());
        assert!(write(&h, "~/work/key", "x", "overwrite").is_err());
        std::os::unix::fs::symlink(h.join(".config/nixe-assistant"), h.join("work/cfg")).unwrap();
        assert!(guard(&h, "~/work/cfg/config.toml", Access::Write).is_err(), "own config through a link");
        // A dangling link as the target of a write resolves through its parent: it must not
        // create the file where the link points.
        std::os::unix::fs::symlink(out.join("made"), h.join("work/dangling")).unwrap();
        let _ = write(&h, "~/work/dangling", "x", "create");
        assert!(!out.join("made").exists(), "wrote through a dangling link");
        assert_eq!(std::fs::read_to_string(h.join(".ssh/id_ed25519")).unwrap(), "KEY");
    }

    #[test]
    fn links_swapped_in_after_the_check_are_not_followed() {
        let (_d, h) = home();
        let target = guard(&h, "~/work/a.txt", Access::Write).unwrap();
        // Something swaps the checked file for a link to a secret before the write runs.
        std::fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink(h.join(".ssh/id_ed25519"), &target).unwrap();
        assert!(read_nofollow(&target).is_err(), "append and backups read through the link");
        assert!(open_nofollow(&target).is_err(), "files.read opens through the link");
        // The atomic write replaces the link itself; the secret is untouched.
        atomic_write(&target, b"new").unwrap();
        assert!(!std::fs::symlink_metadata(&target).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(h.join(".ssh/id_ed25519")).unwrap(), "KEY");
    }

    #[test]
    fn writes_that_run_code() {
        let (_d, h) = home();
        for d in [".local/bin", ".config/hypr", ".config/systemd/user", "repo/.git/hooks"] {
            std::fs::create_dir_all(h.join(d)).unwrap();
        }
        let runs = |input: &str| runs_code(&h, &guard(&h, input, Access::Write).unwrap());
        for input in [
            "~/.bashrc",
            "~/.profile",
            "~/.zshrc",
            "~/.local/bin/nixe-assistant",
            "~/.config/hypr/bindings.lua",
            "~/.config/systemd/user/x.service",
            "~/repo/.git/hooks/pre-commit",
            "~/repo/.git/config",
            "~/.gitconfig",
        ] {
            assert!(runs(input), "{input}");
        }
        for input in ["~/work/a.txt", "~/repo/README.md", "~/work/bashrc", "~/.bashrc.txt"] {
            assert!(!runs(input), "{input}");
        }
        // Through a link: the resolved path decides.
        std::os::unix::fs::symlink(h.join(".local/bin"), h.join("work/bin")).unwrap();
        assert!(runs("~/work/bin/tool"));
        // An undo that would put back a startup file.
        let r = write(&h, "~/.bashrc", "echo hi", "create").unwrap();
        let targets = undo_targets(&h, r["backup"].as_str().unwrap());
        assert!(targets.iter().any(|p| runs_code(&h, p)));
        assert!(undo_targets(&h, "../../etc").is_empty());
    }
}
