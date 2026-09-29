//! The host user, recreated inside the container with the same name, uid,
//! gid, group and home, so that tools which look the user up — ssh first
//! among them — find the same account as on the host.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, ensure};

use crate::constants::{GID_ENV, GROUP_ENV, HOME_ARG, HOME_ENV, UID_ENV, USER_ENV};

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub group: String,
    pub home: PathBuf,
}

impl User {
    /// The user running vz. Names come from `id`, which resolves them the
    /// way the host does, directory services included.
    pub fn of_host() -> anyhow::Result<Self> {
        let home = std::env::var_os(HOME_ENV).context("HOME is not set")?;
        Ok(Self {
            name: id("-un")?,
            uid: id("-u")?.parse().context("parsing `id -u`")?,
            gid: id("-g")?.parse().context("parsing `id -g`")?,
            group: id("-gn")?,
            home: home.into(),
        })
    }

    /// Inside the container: the user the launcher passed in.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            name: env(USER_ENV)?,
            uid: env(UID_ENV)?.parse().context(UID_ENV)?,
            gid: env(GID_ENV)?.parse().context(GID_ENV)?,
            group: env(GROUP_ENV)?,
            home: env(HOME_ENV)?.into(),
        })
    }

    /// The variables that carry this user into the container.
    pub fn env(&self) -> [(&'static str, String); 5] {
        [
            (USER_ENV, self.name.clone()),
            (UID_ENV, self.uid.to_string()),
            (GID_ENV, self.gid.to_string()),
            (GROUP_ENV, self.group.clone()),
            (HOME_ENV, self.home.to_string_lossy().into_owned()),
        ]
    }

    /// The build args a Dockerfile may declare to bake this user into the
    /// image: the runtime variables, with the home as `VZ_HOME`.
    pub fn build_args(&self) -> [(&'static str, String); 5] {
        [
            (USER_ENV, self.name.clone()),
            (UID_ENV, self.uid.to_string()),
            (GID_ENV, self.gid.to_string()),
            (GROUP_ENV, self.group.clone()),
            (HOME_ARG, self.home.to_string_lossy().into_owned()),
        ]
    }

    /// No password (`*`), rather than `x` for one in /etc/shadow: the user has
    /// no shadow entry, and PAM would refuse the account, sudo included.
    pub fn passwd_line(&self, shell: &Path) -> String {
        format!(
            "{}:*:{}:{}::{}:{}",
            self.name,
            self.uid,
            self.gid,
            self.home.display(),
            shell.display()
        )
    }

    pub fn group_line(&self) -> String {
        format!("{}:x:{}:", self.group, self.gid)
    }

    /// Whether the user has to be added. The image may already have it with
    /// the same name, uid and home; any other clash is refused.
    pub fn needs_user(&self, passwd: &str) -> anyhow::Result<bool> {
        let home = self.home.to_string_lossy();
        for entry in entries(passwd) {
            let same_name = entry.name == self.name;
            let same_uid = entry.id == Some(self.uid);
            if same_name && same_uid {
                ensure!(
                    entry.home == Some(&*home),
                    "the image's user `{}` has home {}, the host's is {}; \
                     ssh finds ~/.ssh through the image's",
                    self.name,
                    entry.home.unwrap_or_default(),
                    home
                );
                return Ok(false);
            }
            ensure!(
                !same_uid,
                "the image has user `{}` with uid {}, which clashes with the host's `{}`",
                entry.name,
                self.uid,
                self.name
            );
            ensure!(
                !same_name,
                "the image has user `{}` with uid {}, which clashes with the host's uid {}",
                self.name,
                entry.id.map(|id| id.to_string()).unwrap_or_default(),
                self.uid
            );
        }
        Ok(true)
    }

    /// Whether the group has to be added. The image may already have it with
    /// the same name and gid; any other clash is refused.
    pub fn needs_group(&self, group_file: &str) -> anyhow::Result<bool> {
        for entry in entries(group_file) {
            let same_name = entry.name == self.group;
            let same_gid = entry.id == Some(self.gid);
            if same_name && same_gid {
                return Ok(false);
            }
            ensure!(
                !same_name && !same_gid,
                "the image has group `{}` with gid {}, which clashes with the host's `{}` (gid {})",
                entry.name,
                entry.id.map(|id| id.to_string()).unwrap_or_default(),
                self.group,
                self.gid
            );
        }
        Ok(true)
    }
}

/// A group the user joins besides their own, such as the docker socket's.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraGroup {
    pub name: String,
    pub gid: u32,
}

impl ExtraGroup {
    /// `name:gid`, comma-separated; empty for none.
    pub fn parse_list(text: &str) -> anyhow::Result<Vec<Self>> {
        text.split(',')
            .filter(|item| !item.is_empty())
            .map(|item| {
                let (name, gid) = item
                    .split_once(':')
                    .with_context(|| format!("extra group `{item}` is not `name:gid`"))?;
                let gid = gid
                    .parse()
                    .with_context(|| format!("extra group `{item}` has no numeric gid"))?;
                Ok(Self {
                    name: name.to_owned(),
                    gid,
                })
            })
            .collect()
    }

    /// The `/etc/group` line that gives the gid a name, with `user` as a
    /// member: none if the image already has the gid. The host's name when
    /// the image does not use it, `host-<name>` when it does.
    pub fn line_to_add(&self, group_file: &str, user: &str) -> Option<String> {
        if entries(group_file).any(|entry| entry.id == Some(self.gid)) {
            return None;
        }
        let name_taken = entries(group_file).any(|entry| entry.name == self.name);
        let name = if name_taken {
            format!("host-{}", self.name)
        } else {
            self.name.clone()
        };
        Some(format!("{name}:x:{}:{user}", self.gid))
    }
}

/// `text` with `line` appended on a line of its own.
pub fn with_line(text: &str, line: &str) -> String {
    let separator = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    format!("{text}{separator}{line}\n")
}

/// The name and numeric id of each line in `/etc/passwd` or `/etc/group`,
/// and the home of a `/etc/passwd` line.
struct Entry<'a> {
    name: &'a str,
    id: Option<u32>,
    home: Option<&'a str>,
}

fn entries(text: &str) -> impl Iterator<Item = Entry<'_>> {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.split(':');
            let name = fields.next().unwrap_or_default();
            let id = fields.nth(1).and_then(|id| id.parse().ok());
            let home = fields.nth(2);
            Entry { name, id, home }
        })
}

fn id(flag: &str) -> anyhow::Result<String> {
    let output = Command::new("id")
        .arg(flag)
        .output()
        .with_context(|| format!("running `id {flag}`"))?;
    ensure!(output.status.success(), "`id {flag}` failed");
    Ok(String::from_utf8(output.stdout)
        .with_context(|| format!("`id {flag}` printed non-UTF-8"))?
        .trim_end()
        .to_owned())
}

fn env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| {
        format!("{name} is not set; `vz entrypoint` runs only in a container vz started")
    })
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
                          daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\n";
    const GROUP: &str = "root:x:0:\ndaemon:x:1:\n";

    fn sally() -> User {
        User {
            name: "sally".to_owned(),
            uid: 1000,
            gid: 1000,
            group: "sally".to_owned(),
            home: PathBuf::from("/home/sally"),
        }
    }

    #[test]
    fn passwd_line__user_and_shell__is_passwd_format() {
        let line = sally().passwd_line(Path::new("/bin/bash"));

        assert_eq!(line, "sally:*:1000:1000::/home/sally:/bin/bash");
    }

    #[test]
    fn group_line__user__is_group_format() {
        let line = sally().group_line();

        assert_eq!(line, "sally:x:1000:");
    }

    #[test]
    fn needs_user__absent__is_true() {
        let needed = sally().needs_user(PASSWD).unwrap();

        assert!(needed);
    }

    #[test]
    fn needs_user__same_name_uid_and_home__is_false() {
        let passwd = format!("{PASSWD}sally:x:1000:1000::/home/sally:/bin/bash\n");

        let needed = sally().needs_user(&passwd).unwrap();

        assert!(!needed);
    }

    #[test]
    fn needs_user__same_user_other_home__refuses_naming_both_homes() {
        let passwd = format!("{PASSWD}sally:x:1000:1000::/home/other:/bin/bash\n");

        let error = sally().needs_user(&passwd).unwrap_err();

        assert!(error.to_string().contains("/home/other"), "{error}");
    }

    #[test]
    fn needs_user__uid_taken_by_another__refuses_naming_the_user() {
        let passwd = format!("{PASSWD}ubuntu:x:1000:1000::/home/ubuntu:/bin/bash\n");

        let error = sally().needs_user(&passwd).unwrap_err();

        assert!(error.to_string().contains("`ubuntu`"), "{error}");
    }

    #[test]
    fn needs_user__name_taken_with_another_uid__refuses() {
        let passwd = format!("{PASSWD}sally:x:1001:1001::/home/sally:/bin/sh\n");

        let error = sally().needs_user(&passwd).unwrap_err();

        assert!(error.to_string().contains("uid 1001"), "{error}");
    }

    #[test]
    fn needs_group__absent__is_true() {
        let needed = sally().needs_group(GROUP).unwrap();

        assert!(needed);
    }

    #[test]
    fn needs_group__same_name_and_gid__is_false() {
        let group = format!("{GROUP}sally:x:1000:\n");

        let needed = sally().needs_group(&group).unwrap();

        assert!(!needed);
    }

    #[test]
    fn needs_group__gid_taken_by_another__refuses() {
        let group = format!("{GROUP}ubuntu:x:1000:\n");

        let error = sally().needs_group(&group).unwrap_err();

        assert!(error.to_string().contains("`ubuntu`"), "{error}");
    }

    #[test]
    fn parse_list__forms() {
        let docker = ExtraGroup {
            name: "docker".to_owned(),
            gid: 969,
        };
        let audio = ExtraGroup {
            name: "audio".to_owned(),
            gid: 29,
        };

        assert_eq!(ExtraGroup::parse_list("").unwrap(), vec![]);
        assert_eq!(
            ExtraGroup::parse_list("docker:969").unwrap(),
            vec![docker.clone()]
        );
        assert_eq!(
            ExtraGroup::parse_list("docker:969,audio:29").unwrap(),
            vec![docker, audio]
        );
    }

    #[test]
    fn parse_list__malformed__is_refused_naming_the_item() {
        for text in ["docker", "docker:x"] {
            let error = ExtraGroup::parse_list(text).unwrap_err().to_string();

            assert!(error.contains(&format!("`{text}`")), "{error}");
        }
    }

    #[test]
    fn line_to_add__image_groups() {
        let docker = ExtraGroup {
            name: "docker".to_owned(),
            gid: 969,
        };
        let cases = [
            (GROUP, Some("docker:x:969:sally")),
            ("root:x:0:\nsomething:x:969:\n", None),
            (
                "root:x:0:\ndocker:x:101:\n",
                Some("host-docker:x:969:sally"),
            ),
        ];
        for (group_file, expected) in cases {
            assert_eq!(
                docker.line_to_add(group_file, "sally").as_deref(),
                expected,
                "group file: {group_file:?}"
            );
        }
    }

    #[test]
    fn with_line__trailing_newline_or_not() {
        let cases = [("", "a\n"), ("root\n", "root\na\n"), ("root", "root\na\n")];
        for (text, expected) in cases {
            assert_eq!(with_line(text, "a"), expected, "text: {text:?}");
        }
    }
}
