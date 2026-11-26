//! Text between Slack and agents: what agents write, made safe to post
//! (Slack's special mentions can't ping a channel from it, and its links
//! don't unfurl), in Slack's own markup where a block needs it; what people
//! write in Slack, as plain text for agents; and replies cut to what Slack
//! shows, with a link to the rest.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// The most of an agent's message fctl posts in one Slack message: what a
/// `markdown` block takes is more, but a thread reads better with less, and
/// the rest is a click away.
pub const MAX_REPLY: usize = 3_000;

/// The most a section block's text takes, in Slack's markup.
pub const MAX_SECTION: usize = 3_000;

/// A word joiner: invisible, it keeps Slack from reading what it splits as
/// markup (`<!channel>`, `@here`).
const JOIN: char = '\u{2060}';

/// `text` with Slack's special mentions taken apart, so that whatever an
/// agent writes notifies nobody: `<!channel>`, `<!here>`, `<!everyone>`,
/// user groups (`<!subteam^…>`) and people (`<@U…>`), and `@channel`,
/// `@here` and `@everyone` written out. They read the same.
pub fn defuse(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        match c {
            '<' if matches!(chars.peek(), Some('!' | '@' | '#')) => out.push(JOIN),
            '@' => {
                let rest: String = chars.clone().take(8).collect::<String>().to_lowercase();
                if ["channel", "here", "everyone"]
                    .iter()
                    .any(|w| rest.starts_with(w))
                {
                    out.push(JOIN);
                }
            }
            _ => {}
        }
    }
    out
}

/// `text` as Slack's markup shows it literally: `&`, `<` and `>` escaped,
/// as Slack asks of text fields, so that nothing in it reads as a link or a
/// mention.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A link in Slack's markup, its words escaped.
pub fn link(url: &str, words: &str) -> String {
    let url = url.replace(['|', '>', '<', ' '], "");
    format!("<{url}|{}>", escape(words).replace('|', "¦"))
}

/// What an agent wrote in Markdown, in Slack's own markup (`mrkdwn`), for
/// the blocks that take nothing else: bold, italics, strikes, code, links,
/// quotes and lists as Slack draws them, headings as bold lines, tables as
/// code. Its text is escaped, and its mentions defused.
pub fn mrkdwn(markdown: &str) -> String {
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut out = String::new();
    // Each open list's next number, or None for bullets.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let mut quote = 0usize;
    let mut code_block = false;
    let mut table = false;
    let newline = |out: &mut String, quote: usize| {
        out.push('\n');
        for _ in 0..quote {
            out.push_str("> ");
        }
    };
    for ev in Parser::new_ext(markdown, opts) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { .. } => out.push('*'),
                Tag::BlockQuote(_) => {
                    quote += 1;
                    out.push_str("> ");
                }
                Tag::CodeBlock(_) => {
                    code_block = true;
                    out.push_str("```\n");
                }
                Tag::List(start) => lists.push(start),
                Tag::Item => {
                    let depth = lists.len().saturating_sub(1);
                    out.push_str(&"    ".repeat(depth));
                    match lists.last_mut() {
                        Some(Some(n)) => {
                            out.push_str(&format!("{n}. "));
                            *n += 1;
                        }
                        _ => out.push_str("• "),
                    }
                }
                Tag::Emphasis => out.push('_'),
                Tag::Strong => out.push('*'),
                Tag::Strikethrough => out.push('~'),
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    links.push(dest_url.to_string());
                    out.push('\u{1}');
                }
                Tag::Table(_) => {
                    table = true;
                    out.push_str("```\n");
                }
                Tag::TableHead | Tag::TableRow => {}
                Tag::TableCell => out.push_str("| "),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    newline(&mut out, quote);
                    if lists.is_empty() {
                        newline(&mut out, quote);
                    }
                }
                TagEnd::Heading(_) => {
                    out.push('*');
                    newline(&mut out, quote);
                    newline(&mut out, quote);
                }
                TagEnd::BlockQuote(_) => {
                    quote = quote.saturating_sub(1);
                    newline(&mut out, quote);
                }
                TagEnd::CodeBlock => {
                    code_block = false;
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```");
                    newline(&mut out, quote);
                    newline(&mut out, quote);
                }
                TagEnd::List(_) => {
                    lists.pop();
                    if lists.is_empty() {
                        newline(&mut out, quote);
                    }
                }
                TagEnd::Item => {
                    if !out.ends_with('\n') {
                        newline(&mut out, quote);
                    }
                }
                TagEnd::Emphasis => out.push('_'),
                TagEnd::Strong => out.push('*'),
                TagEnd::Strikethrough => out.push('~'),
                TagEnd::Link | TagEnd::Image => {
                    let url = links.pop().unwrap_or_default();
                    let start = out.rfind('\u{1}').unwrap_or(out.len());
                    let words = out[start + '\u{1}'.len_utf8()..].to_owned();
                    out.truncate(start);
                    let words = words.trim();
                    if url.starts_with("http://") || url.starts_with("https://") {
                        let url = url.replace(['|', '>', '<', ' '], "");
                        out.push_str(&format!(
                            "<{url}|{}>",
                            if words.is_empty() { &url } else { words }
                        ));
                    } else {
                        out.push_str(words);
                    }
                }
                TagEnd::Table => {
                    table = false;
                    out.push_str("```");
                    newline(&mut out, quote);
                    newline(&mut out, quote);
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    out.push_str("|\n");
                }
                TagEnd::TableCell => out.push(' '),
                _ => {}
            },
            Event::Text(t) => {
                let t = escape(&t);
                if code_block || table {
                    out.push_str(&t);
                } else {
                    out.push_str(&t.replace('\n', &format!("\n{}", "> ".repeat(quote))));
                }
            }
            Event::Code(c) => {
                out.push('`');
                out.push_str(&escape(&c).replace('`', "'"));
                out.push('`');
            }
            Event::SoftBreak | Event::HardBreak => newline(&mut out, quote),
            Event::Rule => {
                out.push_str("───");
                newline(&mut out, quote);
            }
            Event::TaskListMarker(done) => out.push_str(if done { "☑ " } else { "☐ " }),
            Event::Html(h) | Event::InlineHtml(h) => out.push_str(&escape(&h)),
            Event::FootnoteReference(f) => out.push_str(&format!("[{f}]")),
            _ => {}
        }
    }
    defuse(collapse(&out).trim())
}

/// At most two line breaks in a row.
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut breaks = 0;
    for c in s.chars() {
        if c == '\n' {
            breaks += 1;
            if breaks > 2 {
                continue;
            }
        } else if c != ' ' || breaks == 0 {
            breaks = 0;
        }
        out.push(c);
    }
    out
}

/// What someone wrote in Slack, as plain text for an agent: mentions as
/// names (`names` gives a user's from their id, where fctl knows it),
/// channels as `#name`, links as their words and address, Slack's escapes
/// undone.
pub fn plain(text: &str, names: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let inner = &after[..close];
        let (target, words) = match inner.split_once('|') {
            Some((t, w)) => (t, Some(w)),
            None => (inner, None),
        };
        let shown = if let Some(user) = target.strip_prefix('@') {
            format!(
                "@{}",
                words
                    .map(str::to_owned)
                    .or_else(|| names(user))
                    .unwrap_or_else(|| user.to_owned())
            )
        } else if let Some(channel) = target.strip_prefix('#') {
            format!("#{}", words.unwrap_or(channel))
        } else if let Some(special) = target.strip_prefix('!') {
            match special.split('^').next().unwrap_or_default() {
                "subteam" => format!("@{}", words.unwrap_or("a group")),
                "date" => words.unwrap_or("a date").to_owned(),
                s => format!("@{s}"),
            }
        } else {
            let url = target.strip_prefix("mailto:").unwrap_or(target);
            match words.filter(|w| *w != target && *w != url) {
                Some(w) => format!("{w} ({url})"),
                None => url.to_owned(),
            }
        };
        out.push_str(&shown);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The ids of the people a message mentions (`<@U…>`), each once.
pub fn mentioned(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for part in text.split("<@").skip(1) {
        let id: String = part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if super::is_id(&id) && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// A message to fctl's app without its mention of the app (`<@UBOT>`), as
/// what they ask: "<@UBOT> fix the tests" asks to fix the tests.
pub fn without_mention(text: &str, bot: &str) -> String {
    text.replace(&format!("<@{bot}>"), "")
        .trim()
        .trim_start_matches([',', ':', '-'])
        .trim()
        .to_owned()
}

/// `text`, at most `max` characters, cut at a paragraph or a line where it
/// can, with the rest a link away (`more`, in Markdown). Whether it was
/// cut.
pub fn cut(text: &str, max: usize, more: &str) -> (String, bool) {
    let text = text.trim();
    if text.chars().count() <= max {
        return (text.to_owned(), false);
    }
    let room = max.saturating_sub(more.chars().count() + 4);
    let head: String = text.chars().take(room).collect();
    let at = head
        .rfind("\n\n")
        .filter(|i| *i > room / 2)
        .or_else(|| head.rfind('\n').filter(|i| *i > room / 2))
        .or_else(|| head.rfind(". ").map(|i| i + 1).filter(|i| *i > room / 2))
        .unwrap_or(head.len());
    let mut body = head[..at].trim_end().to_owned();
    // A code block cut open would swallow the link.
    if body.matches("```").count() % 2 == 1 {
        body.push_str("\n```");
    }
    body.push_str("\n\n");
    body.push_str(more);
    (body, true)
}

/// A line of text at most `max` characters, with `…` where it was cut.
pub fn clip(text: &str, max: usize) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut c: String = one.chars().take(max.saturating_sub(1)).collect();
    c.push('…');
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_an_agent_writes_notify_nobody() {
        let d = defuse("Ping <!channel> and <!here>, <@U123> and @everyone, mail a@here.com");
        assert!(!d.contains("<!channel>") && !d.contains("<!here>") && !d.contains("<@U123>"));
        assert!(!d.contains("@everyone"));
        // They read the same once the word joiners are gone.
        assert_eq!(
            d.replace('\u{2060}', ""),
            "Ping <!channel> and <!here>, <@U123> and @everyone, mail a@here.com"
        );
        assert_eq!(defuse("x <https://a.b|c>"), "x <https://a.b|c>");
    }

    #[test]
    fn markdown_becomes_slacks_markup() {
        let m = mrkdwn(
            "## Done\n\nI fixed **the flaky test** in `auth.rs`, see [#42](https://github.com/acme/web/pull/42).\n\n- one\n- _two_\n\n1. first\n2. second\n\n```rust\nlet a = 1 < 2;\n```\n\n> quoted <!here>",
        );
        assert!(m.starts_with("*Done*\n\n"), "{m}");
        assert!(m.contains("I fixed *the flaky test* in `auth.rs`"), "{m}");
        assert!(
            m.contains("<https://github.com/acme/web/pull/42|#42>"),
            "{m}"
        );
        assert!(m.contains("• one\n• _two_"), "{m}");
        assert!(m.contains("1. first\n2. second"), "{m}");
        assert!(m.contains("```\nlet a = 1 &lt; 2;\n```"), "{m}");
        assert!(m.contains("> quoted &lt;!here&gt;"), "{m}");
        assert!(!m.contains("\n\n\n"));
    }

    #[test]
    fn slacks_markup_becomes_plain_text_for_agents() {
        let names = |id: &str| (id == "U1").then(|| "Ada Lovelace".to_owned());
        assert_eq!(
            plain(
                "<@U1> and <@U2|grace>: see <#C1|deploys>, <https://x.dev/a?b=1&amp;c=2|the doc> &amp; <https://y.dev> <!here> <!subteam^S1|@web>",
                &names
            ),
            "@Ada Lovelace and @grace: see #deploys, the doc (https://x.dev/a?b=1&c=2) & https://y.dev @here @@web"
        );
        assert_eq!(mentioned("<@UBOT> ask <@U1> and <@U1>"), vec!["UBOT", "U1"]);
        assert_eq!(
            without_mention("<@UBOT>: fix the tests", "UBOT"),
            "fix the tests"
        );
        assert_eq!(without_mention("can <@UBOT> help?", "UBOT"), "can  help?");
    }

    #[test]
    fn long_replies_are_cut_with_a_link_to_the_rest() {
        let text = format!("{}\n\n{}", "a".repeat(2000), "b".repeat(2000));
        let (body, was) = cut(&text, MAX_REPLY, "[The rest is in fctl](https://x/s/1)");
        assert!(was);
        assert!(body.chars().count() <= MAX_REPLY);
        assert!(body.ends_with("[The rest is in fctl](https://x/s/1)"));
        assert!(!body.contains('b'));
        let (short, was) = cut("  Done.  ", MAX_REPLY, "x");
        assert_eq!((short.as_str(), was), ("Done.", false));
        let code = format!("```\n{}\n```", "c\n".repeat(3000));
        let (body, _) = cut(&code, 500, "more");
        assert_eq!(body.matches("```").count() % 2, 0);
        assert_eq!(clip("a  b\nc", 10), "a b c");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
    }
}
