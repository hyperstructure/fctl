//! The order people put their workspaces in, and each one's projects
//! (docs/architecture.md#workspaces). Lists and the web app's sidebar show
//! them so. A new order names some of them first; the others follow as they
//! were, and what someone hasn't placed yet comes last, oldest first.

use crate::validate::Result;
use fctl_proto::ApiError;
use std::collections::HashSet;

/// The most names one order takes.
pub const MAX_NAMES: usize = 1000;

/// A new order of `current`, ids with the slugs that name them too, in the
/// order they have now: the ones `wanted` names first, by id or slug, in
/// its order, then the others as they were. A repeated name counts once. A
/// name of none of them is an error on `field`, which says they are `what`
/// ("your workspaces").
pub fn arrange<'a>(
    current: &[(&'a str, &'a str)],
    wanted: &[String],
    field: &str,
    what: &str,
) -> Result<Vec<&'a str>> {
    if wanted.len() > MAX_NAMES {
        return Err(ApiError::validation(
            field,
            format!("Name at most {MAX_NAMES} of {what}"),
        ));
    }
    let mut order = Vec::with_capacity(current.len());
    let mut placed = HashSet::new();
    for name in wanted {
        let name = name.trim();
        let Some(&(id, _)) = current
            .iter()
            .find(|&&(id, slug)| id == name || slug == name)
        else {
            return Err(ApiError::validation(
                field,
                format!("{name} isn't one of {what}"),
            ));
        };
        if placed.insert(id) {
            order.push(id);
        }
    }
    order.extend(
        current
            .iter()
            .map(|&(id, _)| id)
            .filter(|id| !placed.contains(id)),
    );
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: [(&str, &str); 4] = [
        ("prj_1", "web"),
        ("prj_2", "api"),
        ("prj_3", "docs"),
        ("prj_4", "site"),
    ];

    fn arranged(wanted: &[&str]) -> Result<Vec<&'static str>> {
        let wanted: Vec<String> = wanted.iter().map(|s| (*s).to_owned()).collect();
        arrange(&CURRENT, &wanted, "projects", "your projects in Acme")
    }

    #[test]
    fn the_whole_list_in_a_new_order() {
        assert_eq!(
            arranged(&["prj_3", "prj_1", "prj_4", "prj_2"]).ok(),
            Some(vec!["prj_3", "prj_1", "prj_4", "prj_2"])
        );
    }

    #[test]
    fn the_ones_named_go_first_and_the_rest_keep_their_order() {
        assert_eq!(
            arranged(&["prj_4"]).ok(),
            Some(vec!["prj_4", "prj_1", "prj_2", "prj_3"])
        );
        assert_eq!(
            arranged(&["prj_3", "prj_2"]).ok(),
            Some(vec!["prj_3", "prj_2", "prj_1", "prj_4"])
        );
        assert_eq!(
            arranged(&[]).ok(),
            Some(vec!["prj_1", "prj_2", "prj_3", "prj_4"])
        );
    }

    #[test]
    fn slugs_name_them_too() {
        assert_eq!(
            arranged(&["docs", " web ", "prj_2"]).ok(),
            Some(vec!["prj_3", "prj_1", "prj_2", "prj_4"])
        );
    }

    #[test]
    fn a_repeated_name_counts_once() {
        assert_eq!(
            arranged(&["api", "prj_2", "web", "api"]).ok(),
            Some(vec!["prj_2", "prj_1", "prj_3", "prj_4"])
        );
    }

    #[test]
    fn a_name_of_none_of_them_is_an_error() {
        let e = arranged(&["web", "dcos"]).err();
        assert_eq!(
            e.as_ref().and_then(|e| e.field.as_deref()),
            Some("projects")
        );
        assert_eq!(
            e.map(|e| e.message).as_deref(),
            Some("dcos isn't one of your projects in Acme")
        );
    }

    #[test]
    fn at_most_so_many_names() {
        let many = vec!["web".to_owned(); MAX_NAMES + 1];
        assert!(arrange(&CURRENT, &many, "projects", "your projects").is_err());
        let enough = vec!["web".to_owned(); MAX_NAMES];
        assert!(arrange(&CURRENT, &enough, "projects", "your projects").is_ok());
    }

    #[test]
    fn nothing_to_arrange() {
        assert_eq!(
            arrange(&[], &[], "workspaces", "your workspaces").ok(),
            Some(vec![])
        );
    }
}
