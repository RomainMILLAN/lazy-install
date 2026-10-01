//! Who may have written the file we are about to run.
//!
//! The rule is sshd's `StrictModes`, with Debian's user-private-group
//! exception: the file and every directory above it belong to us (or to root
//! for directories), nobody else may write them, and a group may only write them
//! when that group is provably just us. With the umask 002 of this machine,
//! `~/dotfiles` is 775: without the exception, nothing would ever pass.
//!
//! The judgement is a pure function over collected metadata, so it is tested
//! on fabricated chains and never on the disk (where `/tmp`, 1777, would fail
//! every test for the right reason).

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const S_IWGRP: u32 = 0o020;
const S_IWOTH: u32 = 0o002;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Other,
}

/// One path of the chain, as `stat` saw it.
#[derive(Debug, Clone)]
pub struct Node {
    pub path: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub kind: Kind,
}

/// Who we are.
#[derive(Debug, Clone)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
    pub login: String,
}

impl Identity {
    pub fn current() -> Option<Identity> {
        let uid = nix::unistd::geteuid();
        let user = nix::unistd::User::from_uid(uid).ok().flatten()?;
        Some(Identity {
            uid: uid.as_raw(),
            gid: nix::unistd::getegid().as_raw(),
            login: user.name,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GroupEntry {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct UserEntry {
    pub name: String,
    pub gid: u32,
}

/// `/etc/group` and `/etc/passwd`, and nothing else. A machine relying on NSS
/// (sssd, LDAP) could have a remote account sharing our gid that this does not
/// see; the README says so, and `chmod g-w` remains the strict way out.
#[derive(Debug, Clone, Default)]
pub struct GroupDb {
    pub groups: Vec<GroupEntry>,
    pub users: Vec<UserEntry>,
}

impl GroupDb {
    pub fn load() -> Option<GroupDb> {
        Some(GroupDb {
            groups: parse_groups(&fs::read_to_string("/etc/group").ok()?),
            users: parse_users(&fs::read_to_string("/etc/passwd").ok()?),
        })
    }
}

fn parse_groups(text: &str) -> Vec<GroupEntry> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 4 {
                return None;
            }
            Some(GroupEntry {
                name: f[0].to_string(),
                gid: f[2].parse().ok()?,
                members: f[3]
                    .split(',')
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect()
}

fn parse_users(text: &str) -> Vec<UserEntry> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 4 {
                return None;
            }
            Some(UserEntry {
                name: f[0].to_string(),
                gid: f[3].parse().ok()?,
            })
        })
        .collect()
}

/// True if and only if `gid` is our primary group, carries our login as its
/// name, and has nobody else in it — neither listed as a member nor using it as
/// a primary group. Unknown means no.
pub fn is_private_group(gid: u32, me: &Identity, db: &GroupDb) -> bool {
    if gid != me.gid {
        return false;
    }
    let Some(group) = db.groups.iter().find(|g| g.gid == gid) else {
        return false;
    };
    if group.name != me.login {
        return false;
    }
    if group.members.iter().any(|m| *m != me.login) {
        return false;
    }
    !db.users.iter().any(|u| u.gid == gid && u.name != me.login)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrustError {
    #[error("{0} does not exist or cannot be resolved")]
    Unresolvable(PathBuf),
    #[error("{0} is not a regular file")]
    NotAFile(PathBuf),
    #[error("{path} belongs to uid {uid}, not to you")]
    WrongOwner { path: PathBuf, uid: u32 },
    #[error("{0} is writable by others — run: chmod o-w {0}")]
    WritableByOthers(PathBuf),
    #[error("{0} is writable by a shared group — run: chmod g-w {0}")]
    WritableByGroup(PathBuf),
    #[error("cannot read your identity or the group database")]
    NoIdentity,
}

/// The rule, on a chain whose first node is the target and whose following
/// nodes are every directory above it, up to `/`. Pure.
pub fn judge_chain(chain: &[Node], me: &Identity, db: &GroupDb) -> Result<(), TrustError> {
    for (i, node) in chain.iter().enumerate() {
        let is_target = i == 0;
        if is_target && node.kind != Kind::File {
            return Err(TrustError::NotAFile(node.path.clone()));
        }
        let owner_ok = node.uid == me.uid || (!is_target && node.uid == 0);
        if !owner_ok {
            return Err(TrustError::WrongOwner {
                path: node.path.clone(),
                uid: node.uid,
            });
        }
        // Even with the sticky bit: /tmp is refused, as OpenSSH refuses it.
        if node.mode & S_IWOTH != 0 {
            return Err(TrustError::WritableByOthers(node.path.clone()));
        }
        if node.mode & S_IWGRP != 0 && !is_private_group(node.gid, me, db) {
            return Err(TrustError::WritableByGroup(node.path.clone()));
        }
    }
    Ok(())
}

fn node_of(path: &Path) -> Option<Node> {
    let meta = fs::symlink_metadata(path).ok()?;
    let ft = meta.file_type();
    let kind = if ft.is_file() {
        Kind::File
    } else if ft.is_dir() {
        Kind::Dir
    } else {
        Kind::Other
    };
    Some(Node {
        path: path.to_path_buf(),
        uid: meta.uid(),
        gid: meta.gid(),
        mode: meta.mode(),
        kind,
    })
}

/// Collects the chain of an existing path: itself, then each ancestor.
fn collect_chain(canonical: &Path) -> Result<Vec<Node>, TrustError> {
    let mut chain = Vec::new();
    let mut cur = Some(canonical);
    while let Some(p) = cur {
        chain.push(node_of(p).ok_or_else(|| TrustError::Unresolvable(p.to_path_buf()))?);
        cur = p.parent();
    }
    Ok(chain)
}

/// Checks an existing regular file (a script, config.json) against the rule,
/// and returns its canonical path.
pub fn check_file(path: &Path) -> Result<PathBuf, TrustError> {
    let canonical =
        fs::canonicalize(path).map_err(|_| TrustError::Unresolvable(path.to_path_buf()))?;
    let chain = collect_chain(&canonical)?;
    let me = Identity::current().ok_or(TrustError::NoIdentity)?;
    let db = GroupDb::load().unwrap_or_default();
    judge_chain(&chain, &me, &db)?;
    Ok(canonical)
}

/// Checks a directory (the logs directory) the same way, as if it were the
/// first ancestor of a file inside it.
pub fn check_dir(path: &Path) -> Result<PathBuf, TrustError> {
    let canonical =
        fs::canonicalize(path).map_err(|_| TrustError::Unresolvable(path.to_path_buf()))?;
    let mut chain = collect_chain(&canonical)?;
    // A placeholder target that always passes, so the directories are judged
    // with the directory rule (root-owned ancestors allowed).
    let me = Identity::current().ok_or(TrustError::NoIdentity)?;
    chain.insert(
        0,
        Node {
            path: canonical.join("."),
            uid: me.uid,
            gid: me.gid,
            mode: 0o600,
            kind: Kind::File,
        },
    );
    if chain[1].kind != Kind::Dir {
        return Err(TrustError::NotAFile(canonical));
    }
    let db = GroupDb::load().unwrap_or_default();
    judge_chain(&chain, &me, &db)?;
    Ok(canonical)
}

/// Proof that a script passed the trust rule a moment ago.
///
/// Neither `Clone` nor stored anywhere: the concrete runners build it right
/// before creating the process and consume it, so a check made long ago can
/// never be mistaken for a check made now.
#[derive(Debug)]
pub struct TrustedScript {
    canonical: PathBuf,
}

impl TrustedScript {
    pub fn verify(path: &Path) -> Result<TrustedScript, TrustError> {
        check_file(path).map(|canonical| TrustedScript { canonical })
    }

    /// The canonical path, the one handed to bash — never the path as typed.
    pub fn path(&self) -> &Path {
        &self.canonical
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Identity {
        Identity {
            uid: 1001,
            gid: 1001,
            login: "romain".into(),
        }
    }

    fn private_db() -> GroupDb {
        GroupDb {
            groups: vec![GroupEntry {
                name: "romain".into(),
                gid: 1001,
                members: vec![],
            }],
            users: vec![
                UserEntry {
                    name: "romain".into(),
                    gid: 1001,
                },
                UserEntry {
                    name: "root".into(),
                    gid: 0,
                },
            ],
        }
    }

    fn node(path: &str, uid: u32, gid: u32, mode: u32, kind: Kind) -> Node {
        Node {
            path: PathBuf::from(path),
            uid,
            gid,
            mode,
            kind,
        }
    }

    fn chain(file_mode: u32, home_mode: u32) -> Vec<Node> {
        vec![
            node("/home/romain/s.sh", 1001, 1001, file_mode, Kind::File),
            node("/home/romain", 1001, 1001, home_mode, Kind::Dir),
            node("/home", 0, 0, 0o755, Kind::Dir),
            node("/", 0, 0, 0o755, Kind::Dir),
        ]
    }

    #[test]
    fn private_group_tolerates_775() {
        assert_eq!(
            judge_chain(&chain(0o775, 0o775), &me(), &private_db()),
            Ok(())
        );
    }

    #[test]
    fn shared_group_refuses_775_and_names_the_fix() {
        let mut db = private_db();
        db.groups[0].members = vec!["alice".into()];
        let err = judge_chain(&chain(0o664, 0o755), &me(), &db).unwrap_err();
        assert_eq!(err, TrustError::WritableByGroup("/home/romain/s.sh".into()));
        assert!(err.to_string().contains("chmod g-w /home/romain/s.sh"));
    }

    #[test]
    fn others_write_is_refused_even_with_sticky_bit() {
        let c = vec![
            node("/tmp/x/s.sh", 1001, 1001, 0o644, Kind::File),
            node("/tmp/x", 1001, 1001, 0o755, Kind::Dir),
            node("/tmp", 0, 0, 0o1777, Kind::Dir),
            node("/", 0, 0, 0o755, Kind::Dir),
        ];
        let err = judge_chain(&c, &me(), &private_db()).unwrap_err();
        assert_eq!(err, TrustError::WritableByOthers("/tmp".into()));
        assert!(err.to_string().contains("/tmp"));
    }

    #[test]
    fn the_file_must_be_ours_but_ancestors_may_be_root() {
        let mut c = chain(0o644, 0o755);
        c[0].uid = 0;
        assert!(matches!(
            judge_chain(&c, &me(), &private_db()),
            Err(TrustError::WrongOwner { .. })
        ));
        let mut c = chain(0o644, 0o755);
        c[1].uid = 1002;
        assert!(matches!(
            judge_chain(&c, &me(), &private_db()),
            Err(TrustError::WrongOwner { .. })
        ));
    }

    #[test]
    fn target_must_be_a_regular_file() {
        let mut c = chain(0o644, 0o755);
        c[0].kind = Kind::Other;
        assert!(matches!(
            judge_chain(&c, &me(), &private_db()),
            Err(TrustError::NotAFile(_))
        ));
    }

    #[test]
    fn private_group_rules() {
        let db = private_db();
        assert!(is_private_group(1001, &me(), &db));

        let mut renamed = db.clone();
        renamed.groups[0].name = "staff".into();
        assert!(
            !is_private_group(1001, &me(), &renamed),
            "name differs from login"
        );

        let mut member = db.clone();
        member.groups[0].members = vec!["romain".into(), "alice".into()];
        assert!(!is_private_group(1001, &me(), &member), "extra member");

        let mut primary = db.clone();
        primary.users.push(UserEntry {
            name: "bob".into(),
            gid: 1001,
        });
        assert!(
            !is_private_group(1001, &me(), &primary),
            "another account's primary group"
        );

        let mut other_gid = db.clone();
        other_gid.groups.push(GroupEntry {
            name: "romain".into(),
            gid: 2000,
            members: vec![],
        });
        assert!(
            !is_private_group(2000, &me(), &other_gid),
            "not the primary gid"
        );

        assert!(
            !is_private_group(1001, &me(), &GroupDb::default()),
            "unreadable db"
        );
    }

    #[test]
    fn parses_etc_files() {
        let g = parse_groups("romain:x:1001:\nsudo:x:27:romain,alice\n");
        assert_eq!(g[1].members, vec!["romain", "alice"]);
        let u = parse_users("romain:x:1001:1001::/home/romain:/bin/zsh\n");
        assert_eq!(u[0].gid, 1001);
    }
}
