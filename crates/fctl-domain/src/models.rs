//! The model and the effort level a session's Claude works at, as people
//! read them when someone changes them while it runs.

/// A model as the web app's lists name it: Claude Code's aliases for the
/// latest model of a family by the family's name, a model's ID as it is.
pub fn model_label(model: &str) -> &str {
    match model {
        "opus" => "Opus",
        "sonnet" => "Sonnet",
        "haiku" => "Haiku",
        "fable" => "Fable",
        other => other,
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
}

impl ModelName {
    /// Claude starts on `name` (`--model`), or on its default.
    pub fn started(&mut self, name: Option<&str>) {
        self.starting = name.map(str::to_owned);
        self.known = None;
    }

    /// Claude switched to `name`, and named the model it works on now `id`.
    pub fn switched(&mut self, name: &str, id: &str) {
        self.starting = None;
        self.known = Some((name.to_owned(), id.to_owned()));
    }

    /// Claude's `system/init` says it works on the model `id`: the name to
    /// show for it.
    pub fn init(&mut self, id: &str) -> String {
        if let Some(name) = self.starting.take() {
            self.known = Some((name, id.to_owned()));
        }
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

        // Started again on Claude's default: nobody named it.
        m.started(None);
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
            "Noor Haddad changed the model to claude-opus-5-5 and the effort level to Medium"
        );
    }
}
