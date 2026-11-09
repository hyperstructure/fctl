//! The model and the effort level a session's Claude works at, as people
//! read them when someone changes them while it runs.

use std::borrow::Cow;

/// A model as the web app's lists name it: Claude Code's aliases for the
/// latest model of a family by the family's name, a Claude model's undated
/// ID by its family and version (`claude-sonnet-5-5` is Sonnet 5.5),
/// anything else as it is: a snapshot's ID, dated, would read as the undated
/// one's.
pub fn model_label(model: &str) -> Cow<'_, str> {
    match model {
        "opus" => "Opus".into(),
        "sonnet" => "Sonnet".into(),
        "haiku" => "Haiku".into(),
        "fable" => "Fable".into(),
        other => id_label(other).map_or(Cow::Borrowed(other), Cow::Owned),
    }
}

/// A Claude model's undated ID (`claude-opus-4-1`) as its family and
/// version (Opus 4.1).
fn id_label(id: &str) -> Option<String> {
    let rest = id.strip_prefix("claude-")?;
    let (family, version) = rest.split_once('-')?;
    let family = match family {
        "opus" => "Opus",
        "sonnet" => "Sonnet",
        "haiku" => "Haiku",
        "fable" => "Fable",
        _ => return None,
    };
    let digits = |p: &str, n: std::ops::RangeInclusive<usize>| {
        n.contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit())
    };
    let parts: Vec<&str> = version.split('-').collect();
    match parts.as_slice() {
        [major] if digits(major, 1..=3) => Some(format!("{family} {major}")),
        [major, minor] if digits(major, 1..=3) && digits(minor, 1..=2) => {
            Some(format!("{family} {major}.{minor}"))
        }
        _ => None,
    }
}

/// The name a session knows its Claude's model by: the one Claude was
/// started on or switched to (an alias such as `opus`, or a model's ID) for
/// as long as Claude works on the model it took that name for, which its
/// `system/init` and its answer to `set_model` name by ID; the model's ID
/// where nobody named one (Claude's default) or Claude works on another
/// since (`/model` in a message).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelName {
    /// The name Claude was started on, until its `system/init` says which
    /// model that is.
    starting: Option<String>,
    /// A name, and the ID of the model Claude took it for.
    known: Option<(String, String)>,
    /// The ID of the model Claude last said it works on, in this run.
    working: Option<String>,
    /// A switch Claude didn't answer in time: the name asked for, and the
    /// ID of the model Claude worked on then.
    switching: Option<(String, String)>,
}

impl ModelName {
    /// Claude starts on `name` (`--model`), or on its default.
    pub fn started(&mut self, name: Option<&str>) {
        self.starting = name.map(str::to_owned);
        self.known = None;
        self.working = None;
        self.switching = None;
    }

    /// Claude was asked to switch to `name` and didn't say in time whether
    /// it did. Its next `system/init` decides: a model other than the one it
    /// worked on is the one it switched to; the same one, that it didn't
    /// (yet), and a model it switches to later shows by its ID. Before
    /// Claude has named any model, nothing tells which name its first
    /// `system/init` is for, the one it started on or this one, so that
    /// shows by its ID.
    pub fn switching(&mut self, name: &str) {
        self.switching = self.working.clone().map(|from| (name.to_owned(), from));
        if self.working.is_none() {
            self.starting = None;
        }
    }

    /// Claude switched to `name`, and named the model it works on now `id`.
    pub fn switched(&mut self, name: &str, id: &str) {
        self.starting = None;
        self.switching = None;
        self.known = Some((name.to_owned(), id.to_owned()));
        self.working = Some(id.to_owned());
    }

    /// Claude's `system/init` says it works on the model `id`: the name to
    /// show for it.
    pub fn init(&mut self, id: &str) -> String {
        if let Some(name) = self.starting.take() {
            self.known = Some((name, id.to_owned()));
        } else if let Some((name, from)) = self.switching.take()
            && from != id
        {
            self.known = Some((name, id.to_owned()));
        }
        self.working = Some(id.to_owned());
        match &self.known {
            Some((name, known)) if known == id => name.clone(),
            _ => id.to_owned(),
        }
    }
}

/// An effort level as the web app's lists name it.
pub fn effort_label(effort: &str) -> &str {
    match effort {
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Max",
        other => other,
    }
}

/// What a session's transcript says when `by` changes the model, the effort
/// level or both.
pub fn changed(by: &str, model: Option<&str>, effort: Option<&str>) -> String {
    match (model, effort) {
        (Some(m), Some(e)) => format!(
            "{by} changed the model to {} and the effort level to {}",
            model_label(m),
            effort_label(e)
        ),
        (Some(m), None) => format!("{by} changed the model to {}", model_label(m)),
        (None, Some(e)) => format!("{by} changed the effort level to {}", effort_label(e)),
        (None, None) => format!("{by} kept the model and the effort level"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_keeps_the_name_it_was_asked_for_while_claude_works_on_it() {
        let mut m = ModelName::default();
        m.started(Some("opus"));
        assert_eq!(m.init("claude-opus-5-5"), "opus");
        // Each turn's `system/init` names the same model.
        assert_eq!(m.init("claude-opus-5-5"), "opus");
        // `/model claude-haiku-4-5` in a message: another model, by its ID.
        assert_eq!(m.init("claude-haiku-4-5"), "claude-haiku-4-5");
        assert_eq!(m.init("claude-opus-5-5"), "opus");

        m.switched("sonnet", "claude-sonnet-5-5");
        assert_eq!(m.init("claude-sonnet-5-5"), "sonnet");
        assert_eq!(m.init("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(m.init("claude-sonnet-5-5"), "sonnet");

        // A switch Claude didn't answer in time, which its next turn is on.
        m.switching("haiku");
        assert_eq!(m.init("claude-haiku-5-5"), "haiku");
        assert_eq!(m.init("claude-haiku-5-5"), "haiku");

        // Started again on Claude's default: nobody named it.
        m.started(None);
        assert_eq!(m.init("claude-opus-5-5"), "claude-opus-5-5");
        // A switch it didn't answer in time and hasn't made by its next
        // turn: Opus, by its ID, and a later `/model` to Haiku doesn't take
        // the name.
        m.switching("sonnet");
        assert_eq!(m.init("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(m.init("claude-haiku-4-5"), "claude-haiku-4-5");
        // Asked to switch before Claude named a model: nothing tells which
        // name its first model is for.
        m.started(Some("opus"));
        m.switching("sonnet");
        assert_eq!(m.init("claude-sonnet-5-5"), "claude-sonnet-5-5");
        m.started(Some("opus"));
        m.switching("sonnet");
        assert_eq!(m.init("claude-opus-5-5"), "claude-opus-5-5");
        // Started on a model's ID: the ID.
        m.started(Some("claude-opus-5-5"));
        assert_eq!(m.init("claude-opus-5-5"), "claude-opus-5-5");
    }

    #[test]
    fn changes_read_as_the_lists_name_them() {
        assert_eq!(
            changed("Noor Haddad", Some("sonnet"), None),
            "Noor Haddad changed the model to Sonnet"
        );
        assert_eq!(
            changed("Noor Haddad", None, Some("xhigh")),
            "Noor Haddad changed the effort level to Extra high"
        );
        assert_eq!(
            changed("Noor Haddad", Some("claude-opus-5-5"), Some("medium")),
            "Noor Haddad changed the model to Opus 5.5 and the effort level to Medium"
        );
        assert_eq!(
            changed("Noor Haddad", Some("opus[1m]"), None),
            "Noor Haddad changed the model to opus[1m]"
        );
    }

    #[test]
    fn a_model_id_reads_as_its_family_and_version() {
        assert_eq!(model_label("sonnet"), "Sonnet");
        assert_eq!(model_label("claude-sonnet-5-5"), "Sonnet 5.5");
        assert_eq!(model_label("claude-opus-4"), "Opus 4");
        assert_eq!(model_label("claude-haiku-4-5"), "Haiku 4.5");
        for as_is in [
            // A snapshot's ID would read as the undated one's.
            "claude-opus-4-1-20250805",
            "claude-opus-20250514",
            "claude-sonnet-5-5[1m]",
            "claude-3-5-sonnet-20241022",
            "claude-mythos-preview",
            "claude-opus-",
            "claude-opus-5-5-5",
        ] {
            assert_eq!(model_label(as_is), as_is);
        }
    }
}
