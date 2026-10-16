//! Administrative actions the assistant may ask for, and the rules both sides check.
//!
//! The daemon runs as the user. For an admin task it runs `pkexec nixe-assistant-helper
//! VERB ARGS…`; polkit asks for the password (one action per verb, see the policy file),
//! and the helper, running as root, parses the arguments again with [`Action::parse`]
//! and does exactly that one thing. There is no verb that runs a command or writes a
//! file of the caller's choosing.

use std::fmt;

/// Bumped whenever the helper or its policy changes; `install-helper` replaces older ones.
pub const HELPER_VERSION: u32 = 1;
pub const HELPER: &str = "/usr/local/lib/nixe-assistant/nixe-assistant-helper";
pub const POLICY: &str = "/usr/share/polkit-1/actions/io.github.design_nexus.nixe.assistant.policy";
pub const POLICY_XML: &str = include_str!("../data/io.github.design_nexus.nixe.assistant.policy");
/// Root-owned extra deny lists (`deny_packages`, `deny_units`). A user config can't loosen
/// root's rules, so these live outside `~/.config`.
pub const ADMIN_CONFIG: &str = "/etc/nixe-assistant/admin.toml";
pub const LOG: &str = "/var/log/nixe-assistant/privileged.log";
pub const MAX_PACKAGES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

impl ServiceAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "restart" => Some(Self::Restart),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// From the configured repositories only; each entry is `name` or `repo/name`.
    Install { packages: Vec<String> },
    /// Also removes dependencies nothing else needs (`-Rs`); never forced.
    Remove { packages: Vec<String> },
    /// A full system upgrade (`-Syu`).
    Update,
    Service { action: ServiceAction, unit: String },
}

impl Action {
    /// The polkit verb, which is also the helper's first argument.
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Install { .. } => "install",
            Self::Remove { .. } => "remove",
            Self::Update => "update",
            Self::Service { .. } => "service",
        }
    }

    pub fn to_args(&self) -> Vec<String> {
        let mut args = vec![self.verb().to_string()];
        match self {
            Self::Install { packages } | Self::Remove { packages } => args.extend(packages.iter().cloned()),
            Self::Update => {}
            Self::Service { action, unit } => {
                args.push(action.as_str().into());
                args.push(unit.clone());
            }
        }
        args
    }

    /// Parse and check helper arguments. Repository names and the root deny lists are
    /// checked separately ([`check_repo`], [`Deny`]), since they need the system's state.
    pub fn parse(args: &[&str]) -> Result<Self, String> {
        match args {
            ["install", packages @ ..] => Ok(Self::Install { packages: package_list(packages, true)? }),
            ["remove", packages @ ..] => {
                let packages = package_list(packages, false)?;
                if let Some(p) = packages.iter().find(|p| protected_package(p)) {
                    return Err(format!("{p} is part of the base system or the desktop; the assistant never removes it"));
                }
                Ok(Self::Remove { packages })
            }
            ["update"] => Ok(Self::Update),
            ["service", action, unit] => {
                let action = ServiceAction::parse(action).ok_or("the service action must be start, stop or restart")?;
                check_unit(unit)?;
                Ok(Self::Service { action, unit: (*unit).into() })
            }
            [] => Err("no action given".into()),
            [verb, ..] => Err(format!("unknown or malformed action “{verb}”")),
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_args().join(" "))
    }
}

fn package_list(packages: &[&str], repo_allowed: bool) -> Result<Vec<String>, String> {
    if packages.is_empty() {
        return Err("name at least one package".into());
    }
    if packages.len() > MAX_PACKAGES {
        return Err(format!("at most {MAX_PACKAGES} packages at a time"));
    }
    let mut out = Vec::new();
    for p in packages {
        let (repo, name) = match p.split_once('/') {
            Some((repo, name)) if repo_allowed => (Some(repo), name),
            Some(_) => return Err(format!("“{p}” isn't a package name")),
            None => (None, *p),
        };
        if repo.is_some_and(|r| !valid_repo_name(r)) || !valid_package(name) {
            return Err(format!("“{p}” isn't a package name"));
        }
        if !out.iter().any(|o| o == p) {
            out.push((*p).to_string());
        }
    }
    Ok(out)
}

/// pacman's package name rules: lowercase letters, digits and `@._+-`, not starting with
/// `-` or `.`. This also rules out URLs, paths and package files.
pub fn valid_package(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with(['-', '.'])
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "@._+-".contains(c))
        && !name.contains(".pkg.tar")
}

fn valid_repo_name(repo: &str) -> bool {
    !repo.is_empty()
        && repo.len() <= 64
        && !repo.starts_with('-')
        && repo.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
}

/// The repository part of `repo/name`, if it names one pacman has configured.
pub fn check_repo(package: &str, repos: &[String]) -> Result<(), String> {
    match package.split_once('/') {
        Some((repo, _)) if !repos.iter().any(|r| r == repo) => Err(format!("“{repo}” isn't a configured repository")),
        _ => Ok(()),
    }
}

/// Packages the helper never removes: the base system, the kernel and boot chain, and
/// what the desktop needs to come back up.
const PROTECTED_EXACT: &[&str] = &[
    "base", "base-devel", "filesystem", "glibc", "bash", "coreutils", "util-linux", "shadow", "pam", "pambase",
    "pacman", "sudo", "polkit", "mkinitcpio", "dracut", "efibootmgr", "grub", "cryptsetup", "btrfs-progs",
    "networkmanager", "hyprland", "quickshell", "sddm", "pipewire", "wireplumber", "iptables", "nftables",
];
const PROTECTED_FAMILIES: &[&str] = &["linux", "systemd", "dbus", "omarchy", "nvidia", "limine", "mesa"];

pub fn protected_package(name: &str) -> bool {
    let name = name.rsplit('/').next().unwrap_or(name);
    PROTECTED_EXACT.contains(&name)
        || PROTECTED_FAMILIES.iter().any(|f| name == *f || name.strip_prefix(f).is_some_and(|rest| rest.starts_with('-')))
}

/// Only `.service` units, named the way systemd names them. Targets, sockets and timers
/// pull in or stop many things at once, which is too broad for a single approval.
pub fn check_unit(unit: &str) -> Result<(), String> {
    let ok = unit
        .strip_suffix(".service")
        .is_some_and(|name| {
            !name.is_empty()
                && name.len() <= 120
                && !name.starts_with(['-', '.', '@'])
                && name.chars().all(|c| c.is_ascii_alphanumeric() || "@._-:\\".contains(c))
        });
    if !ok {
        return Err(format!("“{unit}” isn't a service unit name"));
    }
    if critical_unit(unit) {
        return Err(format!("{unit} keeps the system or the session running; the assistant doesn't control it"));
    }
    Ok(())
}

/// Units that would take the session, the bus, logins or polkit itself down with them.
pub fn critical_unit(unit: &str) -> bool {
    let name = unit.strip_suffix(".service").unwrap_or(unit);
    let template = name.split('@').next().unwrap_or(name);
    name.starts_with("dbus")
        || name.starts_with("systemd-")
        || name.starts_with("nixe-")
        || matches!(
            name,
            "polkit" | "display-manager" | "sddm" | "gdm" | "lightdm" | "greetd" | "ly" | "emergency" | "rescue"
                | "init" | "auditd"
        )
        || matches!(template, "user" | "user-runtime-dir" | "getty" | "autovt" | "serial-getty" | "container-getty")
}

/// The root-owned extra deny lists in [`ADMIN_CONFIG`].
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct Deny {
    pub deny_packages: Vec<String>,
    pub deny_units: Vec<String>,
}

impl Deny {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("{ADMIN_CONFIG}: {e}"))
    }

    pub fn check(&self, action: &Action) -> Result<(), String> {
        match action {
            Action::Install { packages } | Action::Remove { packages } => {
                for p in packages {
                    let name = p.rsplit('/').next().unwrap_or(p);
                    if self.deny_packages.iter().any(|d| d == name) {
                        return Err(format!("{name} is denied in {ADMIN_CONFIG}"));
                    }
                }
                Ok(())
            }
            Action::Service { unit, .. } if self.deny_units.iter().any(|d| d == unit) => {
                Err(format!("{unit} is denied in {ADMIN_CONFIG}"))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for args in [
            &["install", "sl"][..],
            &["install", "extra/sl", "cowsay"],
            &["remove", "sl"],
            &["update"],
            &["service", "restart", "cups.service"],
            &["service", "stop", "getty-helper@tty1.service"],
        ] {
            let action = Action::parse(args).unwrap();
            assert_eq!(action.to_args(), args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        }
        assert_eq!(Action::parse(&["install", "sl", "sl"]).unwrap(), Action::Install { packages: vec!["sl".into()] });
    }

    #[test]
    fn rejects() {
        let eleven: Vec<String> = (0..11).map(|i| format!("p{i}")).collect();
        let eleven: Vec<&str> = std::iter::once("install").chain(eleven.iter().map(String::as_str)).collect();
        for args in [
            &[][..],
            &["install"],
            &["remove"],
            &["update", "now"],
            &["shell", "id"],
            &["install", "-Syu"],
            &["install", "--overwrite=*"],
            &["install", "sl; rm -rf /"],
            &["install", "../etc/passwd"],
            &["install", "/tmp/x.pkg.tar.zst"],
            &["install", "x.pkg.tar.zst"],
            &["install", "https://example.com/x.pkg.tar.zst"],
            &["install", "Upper"],
            &["install", ".hidden"],
            &["install", "-/sl"],
            &["install", "a/b/c"],
            &["remove", "extra/sl"],
            &["remove", "-Rdd"],
            &["remove", "glibc"],
            &["remove", "sl", "linux"],
            &["service", "kill", "cups.service"],
            &["service", "restart", "cups"],
            &["service", "restart", "multi-user.target"],
            &["service", "restart", "a b.service"],
            &["service", "restart", "--now.service"],
            &["service", "restart", "dbus-broker.service"],
            &["service", "stop", "dbus.service"],
            &["service", "stop", "systemd-logind.service"],
            &["service", "stop", "polkit.service"],
            &["service", "stop", "sddm.service"],
            &["service", "stop", "display-manager.service"],
            &["service", "stop", "user@1000.service"],
            &["service", "stop", "getty@tty1.service"],
            &["service", "stop", "nixe-assistantd.service"],
            &["service", "restart"],
            // Look-alikes, control characters and smuggled flags.
            &["install", "\u{0455}l"],
            &["install", "sl\n"],
            &["install", "sl\0"],
            &["install", "sl --noconfirm"],
            &["install", "sl\t-Syu"],
            &["install", "$(id)"],
            &["install", "`id`"],
            &["remove", "linux-lts"],
            &["remove", "omarchy"],
            &["service", "restart", "../../x.service"],
            &["service", "restart", "-H.service"],
            &["service", "restart", "cups.service\n"],
            &["service", "restart", "cups.service --root=/"],
            &["service", "restart", "c\u{0443}ps.service"],
            &eleven,
        ] {
            assert!(Action::parse(args).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn protected() {
        for p in ["linux", "linux-lts", "linux-firmware", "systemd", "systemd-libs", "glibc", "pacman", "base", "core/base", "omarchy-keyring", "nvidia-open-dkms", "dbus-broker"] {
            assert!(protected_package(p), "{p}");
        }
        for p in ["sl", "linuxconsole", "cowsay", "systemdgenie", "base64"] {
            assert!(!protected_package(p), "{p}");
        }
    }

    #[test]
    fn repos_and_deny() {
        let repos = vec!["core".to_string(), "extra".to_string()];
        assert!(check_repo("extra/sl", &repos).is_ok() && check_repo("sl", &repos).is_ok());
        assert!(check_repo("chaotic-aur/sl", &repos).is_err());
        let deny = Deny::parse("deny_packages = [\"steam\"]\ndeny_units = [\"sshd.service\"]").unwrap();
        assert!(deny.check(&Action::parse(&["install", "multilib/steam"]).unwrap()).is_err());
        assert!(deny.check(&Action::parse(&["service", "restart", "sshd.service"]).unwrap()).is_err());
        assert!(deny.check(&Action::parse(&["service", "restart", "cups.service"]).unwrap()).is_ok());
        assert!(Deny::parse("deny_units = 3").is_err());
    }

    #[test]
    fn policy_has_one_action_per_verb() {
        for verb in ["install", "remove", "update", "service"] {
            assert!(POLICY_XML.contains(&format!("io.github.design_nexus.nixe.assistant.{verb}\"")));
            assert!(POLICY_XML.contains(&format!("exec.argv1\">{verb}<")));
        }
        assert_eq!(POLICY_XML.matches(HELPER).count(), 4);
    }
}
