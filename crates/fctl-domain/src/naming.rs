//! Derived names: session titles, slugs, branch names.

/// A title from the first meaningful line of a prompt, at most 72 characters.
pub fn title_from_prompt(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['#', '>', '-', '*', ' '])
                .trim()
        })
        .find(|l| !l.is_empty())
        .unwrap_or("Untitled session");
    truncate_words(line, 72)
}

/// A title for a task that is only files: `screenshot.png`, or
/// `screenshot.png and 2 more files`.
pub fn title_from_files(names: &[&str]) -> String {
    match names {
        [] => "Untitled session".to_owned(),
        [one] => truncate_words(one, 72),
        [first, rest @ ..] => {
            let more = if rest.len() == 1 { "file" } else { "files" };
            truncate_words(&format!("{first} and {} more {more}", rest.len()), 72)
        }
    }
}

/// Truncates at a word boundary, adding `…` when shortened.
pub fn truncate_words(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    let trimmed = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max_chars / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!(
        "{}…",
        trimmed.trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
    )
}

/// `Add OAuth login!` → `add-oauth-login`, at most `max` bytes.
pub fn slugify(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        let c = fold_accent(c);
        if c.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
        if out.len() >= max {
            break;
        }
    }
    out.truncate(max);
    out.trim_end_matches('-').to_owned()
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

/// The default working branch of a session: `fctl/<number>-<slug of title>`.
pub fn session_branch(number: u32, title: &str) -> String {
    let slug = slugify(title, 40);
    if slug.is_empty() {
        format!("fctl/{number}")
    } else {
        format!("fctl/{number}-{slug}")
    }
}

/// A project slug from its name (callers check uniqueness).
pub fn project_slug(name: &str) -> String {
    let s = slugify(name, 48);
    if s.len() < 2 {
        format!("project-{s}").trim_end_matches('-').to_owned()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_from_files() {
        assert_eq!(title_from_files(&["shot.png"]), "shot.png");
        assert_eq!(
            title_from_files(&["a.png", "b.pdf"]),
            "a.png and 1 more file"
        );
        assert_eq!(
            title_from_files(&["a.png", "b", "c"]),
            "a.png and 2 more files"
        );
        assert_eq!(title_from_files(&[]), "Untitled session");
    }

    #[test]
    fn titles() {
        assert_eq!(
            title_from_prompt("\n\n## Fix the login bug\nDetails…"),
            "Fix the login bug"
        );
        assert_eq!(title_from_prompt("   "), "Untitled session");
        let long =
            "Refactor the session manager so that sandboxes are reused across runs and cleaned up";
        let t = title_from_prompt(long);
        assert!(t.chars().count() <= 72, "{t}");
        assert!(t.ends_with('…'));
    }

    #[test]
    fn slugs_and_branches() {
        assert_eq!(slugify("Add OAuth login!", 40), "add-oauth-login");
        assert_eq!(slugify("Über café", 40), "uber-cafe");
        assert_eq!(
            session_branch(12, "Fix: the *login* bug"),
            "fctl/12-fix-the-login-bug"
        );
        assert_eq!(session_branch(3, "!!!"), "fctl/3");
        assert_eq!(project_slug("X"), "project-x");
        assert!(slugify(&"word ".repeat(30), 40).len() <= 40);
    }
}
