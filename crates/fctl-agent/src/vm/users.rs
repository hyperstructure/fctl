//! Who a program runs as: the image's user, found as `docker exec` finds it
//! in the image's `/etc/passwd` and `/etc/group`.

/// A user to run a program as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    /// Its supplementary groups.
    pub groups: Vec<u32>,
    pub home: String,
}

/// The user `spec` names (`name`, `uid`, `name:group` or `uid:gid`; unset:
/// root), from this system's files.
pub fn find(spec: Option<&str>) -> Result<User, String> {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
    find_in(spec, &passwd, &group)
}

/// A line of `/etc/passwd`: name, uid, gid, home.
fn accounts(passwd: &str) -> impl Iterator<Item = (&str, u32, u32, &str)> {
    passwd.lines().filter_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        match f.as_slice() {
            [name, _, uid, gid, _, home, ..] => {
                Some((*name, uid.parse().ok()?, gid.parse().ok()?, *home))
            }
            _ => None,
        }
    })
}

/// A line of `/etc/group`: name, gid, members.
fn groups(group: &str) -> impl Iterator<Item = (&str, u32, Vec<&str>)> {
    group.lines().filter_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        match f.as_slice() {
            [name, _, gid, members, ..] => Some((
                *name,
                gid.parse().ok()?,
                members.split(',').filter(|m| !m.is_empty()).collect(),
            )),
            [name, _, gid] => Some((*name, gid.parse().ok()?, Vec::new())),
            _ => None,
        }
    })
}

/// [`find`], from the files' text. A uid that no account has runs with
/// group 0 and `/` as its home, as with Docker.
pub fn find_in(spec: Option<&str>, passwd: &str, group: &str) -> Result<User, String> {
    let spec = spec
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("root");
    let (user, group_spec) = match spec.split_once(':') {
        Some((u, g)) => (u, Some(g)),
        None => (spec, None),
    };
    let number = user.parse::<u32>().ok();
    let account = accounts(passwd).find(|(name, uid, _, _)| match number {
        Some(n) => *uid == n,
        None => *name == user,
    });
    let (name, uid, mut gid, home) = match (account, number) {
        (Some((name, uid, gid, home)), _) => (Some(name), uid, gid, home.to_owned()),
        (None, Some(n)) => (None, n, 0, "/".to_owned()),
        (None, None) if user == "root" => (Some("root"), 0, 0, "/root".to_owned()),
        (None, None) => {
            return Err(format!(
                "unable to find user {user}: no matching entries in passwd file"
            ));
        }
    };
    if let Some(g) = group_spec {
        gid = match g.parse::<u32>() {
            Ok(n) => n,
            Err(_) => groups(group)
                .find(|(n, _, _)| *n == g)
                .map(|(_, gid, _)| gid)
                .ok_or_else(|| {
                    format!("unable to find group {g}: no matching entries in group file")
                })?,
        };
    }
    let mut supplementary = vec![gid];
    if let Some(name) = name {
        for (_, g, members) in groups(group) {
            if members.contains(&name) && !supplementary.contains(&g) {
                supplementary.push(g);
            }
        }
    }
    Ok(User {
        uid,
        gid,
        groups: supplementary,
        home: if home.is_empty() { "/".into() } else { home },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str =
        "root:x:0:0:root:/root:/bin/bash\nagent:x:1000:1000::/home/agent:/bin/bash\nbroken line\n";
    const GROUP: &str = "root:x:0:\nsudo:x:27:agent\nagent:x:1000:\ndocker:x:999:other,agent\n";

    #[test]
    fn finds_users_as_docker_does() {
        assert_eq!(
            find_in(Some("agent"), PASSWD, GROUP),
            Ok(User {
                uid: 1000,
                gid: 1000,
                groups: vec![1000, 27, 999],
                home: "/home/agent".into()
            })
        );
        // By uid, with another group.
        let u = find_in(Some("1000:sudo"), PASSWD, GROUP);
        assert_eq!(
            u.map(|u| (u.uid, u.gid, u.groups)),
            Ok((1000, 27, vec![27, 999]))
        );
        // Root, also with no files at all.
        for (passwd, group) in [(PASSWD, GROUP), ("", "")] {
            let root = find_in(None, passwd, group);
            assert_eq!(
                root.map(|u| (u.uid, u.gid, u.home)),
                Ok((0, 0, "/root".into()))
            );
        }
        // A uid no account has.
        let u = find_in(Some("4242"), PASSWD, GROUP);
        assert_eq!(u.map(|u| (u.uid, u.gid, u.home)), Ok((4242, 0, "/".into())));
        assert_eq!(
            find_in(Some("ghost"), PASSWD, GROUP),
            Err("unable to find user ghost: no matching entries in passwd file".into())
        );
        assert!(find_in(Some("agent:nogroup"), PASSWD, GROUP).is_err());
    }
}
