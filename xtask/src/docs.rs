//! Checks for `docs/`: every link in the docs and in `docs/llms.txt` leads
//! somewhere, a file of this repository, a heading of a doc, or a page
//! fctl.sh serves from `docs/` (`https://fctl.sh/docs/<page>.md`), so that
//! people and agents who follow them find what they say. fctl.sh renders
//! the docs from a commit of this repository (hyperstructure/fctl-site).

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    /// The repository's root.
    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    /// The link targets of Markdown text: `[text](target)` outside code,
    /// and the bare URLs of this repository and fctl.sh's docs that prose
    /// names.
    fn links(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut fenced = false;
        // Inline code says commands and paths, not links. It can go on
        // over a line break, but not past the end of a paragraph.
        let mut in_code = false;
        for line in text.lines() {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
                in_code = false;
                continue;
            }
            if fenced {
                continue;
            }
            if line.trim().is_empty() {
                in_code = false;
            }
            let mut prose = String::new();
            for c in line.chars() {
                if c == '`' {
                    in_code = !in_code;
                    prose.push(' ');
                } else if !in_code {
                    prose.push(c);
                }
            }
            let mut rest = prose.as_str();
            while let Some(at) = rest.find("](") {
                let after = &rest[at + 2..];
                let end = after.find(')').unwrap_or(after.len());
                out.push(after[..end].to_owned());
                rest = &after[end..];
            }
            for word in prose.split(|c: char| c.is_whitespace() || c == '(' || c == ')') {
                let word = word.trim_end_matches(['.', ',', ':', ';']);
                if word.starts_with("https://fctl.sh/docs/")
                    || word.starts_with("https://github.com/hyperstructure/fctl/")
                {
                    out.push(word.to_owned());
                }
            }
        }
        out
    }

    /// A heading's anchor as GitHub makes it: lowercase, without
    /// punctuation, spaces as hyphens.
    fn slug(heading: &str) -> String {
        let text = heading
            .replace('`', "")
            .replace("](", " ")
            .replace(['[', ']'], "");
        text.trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
            .map(|c| if c == ' ' { '-' } else { c })
            .collect()
    }

    /// The anchors of a Markdown file's headings, numbered as GitHub numbers
    /// the ones that repeat (`operating-it`, `operating-it-1`).
    fn anchors(text: &str) -> Vec<String> {
        let mut seen: HashMap<String, u32> = HashMap::new();
        let mut out = Vec::new();
        let mut fenced = false;
        for line in text.lines() {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
                continue;
            }
            let Some(heading) = line.strip_prefix('#').filter(|_| !fenced) else {
                continue;
            };
            let heading = heading.trim_start_matches('#');
            if !heading.starts_with(' ') {
                continue;
            }
            let base = slug(heading);
            let n = seen.entry(base.clone()).or_insert(0);
            out.push(match *n {
                0 => base,
                k => format!("{base}-{k}"),
            });
            *n += 1;
        }
        out
    }

    /// Why a link leads nowhere, or `None` when it leads somewhere.
    fn broken(root: &Path, from: &Path, target: &str) -> Option<String> {
        let (path, anchor) = match target.split_once('#') {
            Some((p, a)) => (p, Some(a)),
            None => (target, None),
        };
        let file = if let Some(page) = path
            .strip_prefix("https://fctl.sh/docs/")
            .and_then(|p| p.strip_suffix(".md"))
        {
            match page {
                "index" => root.join("docs/README.md"),
                page => root.join("docs").join(format!("{page}.md")),
            }
        } else if let Some(p) = path
            .strip_prefix("https://github.com/hyperstructure/fctl/blob/main/")
            .or_else(|| path.strip_prefix("https://github.com/hyperstructure/fctl/tree/main/"))
        {
            root.join(p)
        } else if path.contains(':') || path.starts_with('<') {
            // Other sites, mailto:, and templates such as <page>.
            return None;
        } else if path.is_empty() {
            from.to_path_buf()
        } else {
            from.parent().unwrap_or(root).join(path)
        };
        if !file.exists() {
            return Some(format!("{} doesn't exist", file.display()));
        }
        let anchor = anchor.filter(|a| !a.is_empty())?;
        if file.extension().is_none_or(|e| e != "md") {
            return None;
        }
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        (!anchors(&text).iter().any(|a| a == anchor))
            .then(|| format!("{} has no heading #{anchor}", file.display()))
    }

    #[test]
    fn every_link_in_the_docs_leads_somewhere() {
        let root = root();
        let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("docs"))
            .unwrap()
            .chain(std::fs::read_dir(root.join("docs/adr")).unwrap())
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "md" || e == "txt"))
            .collect();
        files.sort();
        assert!(files.iter().any(|f| f.ends_with("llms.txt")));
        let mut problems = std::collections::BTreeSet::new();
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            for target in links(&text) {
                if let Some(why) = broken(&root, file, &target) {
                    let name = file.strip_prefix(&root).unwrap_or(file).display();
                    problems.insert(format!("{name}: {target}: {why}"));
                }
            }
        }
        let problems: Vec<String> = problems.into_iter().collect();
        assert!(
            problems.is_empty(),
            "broken links:\n{}",
            problems.join("\n")
        );
    }

    /// llms.txt has the shape https://llmstxt.org asks for: the name as its
    /// heading, then a summary in a quote, and absolute links throughout,
    /// since it is served at fctl.sh, every instance and GitHub alike.
    #[test]
    fn llms_txt_is_an_llms_txt() {
        let text = std::fs::read_to_string(root().join("docs/llms.txt")).unwrap();
        let mut lines = text.lines().filter(|l| !l.is_empty());
        assert_eq!(lines.next(), Some("# fctl"));
        assert!(lines.next().is_some_and(|l| l.starts_with("> ")));
        assert_eq!(text.lines().filter(|l| l.starts_with("# ")).count(), 1);
        let relative: Vec<String> = links(&text)
            .into_iter()
            .filter(|t| !t.starts_with("https://"))
            .collect();
        assert!(
            relative.is_empty(),
            "links that aren't absolute: {relative:?}"
        );
    }

    #[test]
    fn finds_links_and_anchors_as_github_does() {
        let text = "See [the CLI](cli.md#hosts) and `[not](code.md)`.\n\
                    ```sh\n[nor](this.md)\n```\n\
                    More at https://fctl.sh/docs/api.md#errors, and `fctl\n\
                    run` in [the docs](agents.md).\n\
                    \n\
                    A stray ` ends with its paragraph.\n\
                    \n\
                    [After it](security.md)";
        assert_eq!(
            links(text),
            [
                "cli.md#hosts",
                "https://fctl.sh/docs/api.md#errors",
                "agents.md",
                "security.md"
            ]
        );
        assert_eq!(slug(" Confirming it's you"), "confirming-its-you");
        assert_eq!(
            slug(" Connect GitHub (optional, recommended)"),
            "connect-github-optional-recommended"
        );
        assert_eq!(
            slug(" 5. Bring a machine online"),
            "5-bring-a-machine-online"
        );
        assert_eq!(
            slug(" `fctl ticket` in [a sandbox](x.md)"),
            "fctl-ticket-in-a-sandbox-xmd"
        );
        assert_eq!(
            anchors("## Operating it\n```\n# not a heading\n```\n## Operating it\n#nope\n"),
            ["operating-it", "operating-it-1"]
        );
    }
}
