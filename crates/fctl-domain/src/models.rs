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
