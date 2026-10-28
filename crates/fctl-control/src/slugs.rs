//! Workspaces' and projects' slugs, their addresses (`/w/<slug>`,
//! `/p/<slug>`): which are free, and the ones instance admins reserve for
//! later (`reserved_slugs`, `fctl_domain::slugs`).

use crate::db::{Count, q, s};
use crate::error::{AppResult, conflict, validation};
use fctl_domain::{slugs, validate};
use worker::D1Database;

/// What a slug is the address of. Workspaces and projects each have their
/// own: a workspace and a project can have the same one.
#[derive(Clone, Copy)]
pub enum Of {
    Workspace,
    Project,
}

impl Of {
    fn table(self) -> &'static str {
        match self {
            Self::Workspace => "workspaces",
            Self::Project => "projects",
        }
    }

    fn taken(self) -> &'static str {
        match self {
            Self::Workspace => "Another workspace uses this address",
            Self::Project => "Another project uses this name",
        }
    }
}

/// Whether a workspace or project other than `except` has `slug`.
pub async fn taken(db: &D1Database, of: Of, slug: &str, except: Option<&str>) -> AppResult<bool> {
    let c: Option<Count> = q(
        db,
        &format!(
            "SELECT COUNT(*) AS n FROM {} WHERE slug = ? AND id != ?",
            of.table()
        ),
        &[s(slug), s(except.unwrap_or(""))],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| c.n) > 0)
}

/// Whether a workspace or project has `slug`, or an admin reserves it.
async fn unavailable(db: &D1Database, of: Of, slug: &str) -> AppResult<bool> {
    let c: Option<Count> = q(
        db,
        &format!(
            "SELECT (SELECT COUNT(*) FROM {} WHERE slug = ?1) + (SELECT COUNT(*) FROM reserved_slugs WHERE slug = ?1) AS n",
            of.table()
        ),
        &[s(slug)],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| c.n) > 0)
}

/// The slug someone chose for a workspace or project (`except`: the
/// workspace that changes its own): not one another has, nor a reserved
/// one, of fctl's or an admin's.
pub async fn chosen(
    db: &D1Database,
    of: Of,
    input: &str,
    except: Option<&str>,
) -> AppResult<String> {
    let slug = validate::slug(input)?;
    if taken(db, of, &slug, except).await? {
        return Err(validation("slug", of.taken()));
    }
    let reserved: Option<Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM reserved_slugs WHERE slug = ?",
        &[s(&slug)],
    )?
    .first()
    .await?;
    if reserved.map_or(0, |c| c.n) > 0 {
        return Err(validation("slug", "This name is reserved"));
    }
    Ok(slug)
}

/// The slug of a new workspace or project, from the one its name makes:
/// that one while it is free, else with a random ending (`example-829d34`)
/// when another has it or it is reserved, by fctl or an admin.
pub async fn free(db: &D1Database, of: Of, base: &str) -> AppResult<String> {
    if let Ok(slug) = validate::slug(base)
        && !unavailable(db, of, &slug).await?
    {
        return Ok(slug);
    }
    // 16.7 million endings: another try is for the odd one taken.
    for _ in 0..5 {
        let candidate = slugs::with_ending(base, fctl_domain::random_bytes());
        if !unavailable(db, of, &candidate).await? {
            return Ok(candidate);
        }
    }
    Err(conflict(match of {
        Of::Workspace => "Pick a different workspace name",
        Of::Project => "Pick a different project name",
    }))
}
