//! Process ancestry via `/proc`.

use std::fs;

/// Parent PID of `pid`, read from `/proc/<pid>/stat`.
pub fn parent_of(pid: i32) -> Option<i32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_ppid(&stat)
}

/// Field 4 of `/proc/<pid>/stat`. The command name (field 2) may contain
/// spaces and parentheses, so parse from the last `)`.
fn parse_ppid(stat: &str) -> Option<i32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Whether `pid` is `ancestor` or one of its descendants.
pub fn is_descendant(pid: i32, ancestor: i32, parent_of: impl Fn(i32) -> Option<i32>) -> bool {
    let mut current = pid;
    // Bounded in case of a cycle from a racy read.
    for _ in 0..1024 {
        if current == ancestor {
            return true;
        }
        match parent_of(current) {
            Some(parent) if parent > 0 && parent != current => current = parent,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn parses_stat_with_odd_comm() {
        assert_eq!(
            parse_ppid("1234 (we (ird) name) S 42 1234 1234 0"),
            Some(42)
        );
        assert_eq!(parse_ppid("garbage"), None);
    }

    #[test]
    fn ancestry() {
        let tree: HashMap<i32, i32> = [(50, 40), (40, 30), (30, 1), (60, 1)].into();
        let parent = |p| tree.get(&p).copied();
        assert!(is_descendant(50, 30, parent));
        assert!(is_descendant(30, 30, parent));
        assert!(!is_descendant(60, 30, parent));
        assert!(!is_descendant(99, 30, parent));
    }

    #[test]
    fn real_proc_self() {
        let me = std::process::id() as i32;
        let parent = parent_of(me).unwrap();
        assert!(is_descendant(me, parent, parent_of));
    }
}
