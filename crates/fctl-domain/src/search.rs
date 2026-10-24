//! Searching sessions: what of a session is searchable, how a search
//! compares with text, and the snippets results show.
//!
//! Each session's title, its description, people's messages and the
//! agent's replies are searchable, each as a document of its own (a
//! [`Doc`]): the compact part of a transcript that says what it is about,
//! without the agent's tool calls, their output or its thinking. A search
//! finds the documents that contain what was typed, as typed, but for case,
//! accents and how much whitespace separates words. Control planes keep
//! them in each workspace's search index (`SearchObject`), from which a
//! result names the session and the event (`seq`) its page opens at.

use fctl_proto::ApiError;
use fctl_proto::api::{SearchMatch, SearchMatchKind};
use fctl_proto::event::{EventBody, UserMessage};
use fctl_proto::session::UserRef;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The shortest search, in characters: fewer find too much to be of use,
/// but in scripts that write words in two ([`dense`]), where two do.
pub const MIN_QUERY_CHARS: usize = 3;
/// The longest search, in characters.
pub const MAX_QUERY_CHARS: usize = 200;
/// The most of a message or a reply that is searchable, in characters: its
/// start, which says what it is about. Longer ones are rare (pasted logs,
/// long reports).
pub const MAX_DOC_CHARS: usize = 10_000;
/// The most sessions a search returns.
pub const MAX_RESULTS: u32 = 50;
/// The most matches a result shows of its session.
pub const MATCHES_SHOWN: usize = 3;
/// About how many characters of a document a snippet shows.
pub const SNIPPET_CHARS: usize = 160;
/// About how many characters a snippet shows before its first match.
const SNIPPET_LEAD: usize = 48;
/// How many characters past its length a snippet looks for a space to end
/// at, before it cuts a word.
const SNIPPET_SLACK: usize = 20;

/// What a document of a session is: its title or description (`seq` 0), a
/// message people (or fctl) wrote to the agent, or the agent's reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Title,
    Description,
    Message,
    Reply,
}

impl DocKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Description => "description",
            Self::Message => "message",
            Self::Reply => "reply",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "title" => Self::Title,
            "description" => Self::Description,
            "message" => Self::Message,
            "reply" => Self::Reply,
            _ => return None,
        })
    }

    pub fn to_match_kind(self) -> SearchMatchKind {
        match self {
            Self::Title => SearchMatchKind::Title,
            Self::Description => SearchMatchKind::Description,
            Self::Message => SearchMatchKind::Message,
            Self::Reply => SearchMatchKind::Reply,
        }
    }
}

/// A searchable document of a session: its text, normalized ([`normalize`]),
/// and where it is: the event (`seq`; 0 for the title and description) and
/// which of the event's documents it is (`part`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doc {
    pub seq: i64,
    pub part: u32,
    pub kind: DocKind,
    /// When it was written (ms); the session's start for its title.
    pub ts: i64,
    /// Who wrote a message; absent for fctl's own and the agent's replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<UserRef>,
    pub text: String,
}

/// Text as it is searched: runs of whitespace, line breaks too, become one
/// space, and it starts and ends with neither. What a search finds doesn't
/// depend on how a message was wrapped.
pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// [`normalize`]d text cut to [`MAX_DOC_CHARS`].
fn doc_text(text: &str) -> String {
    let text = normalize(text);
    match text.char_indices().nth(MAX_DOC_CHARS) {
        Some((cut, _)) => text[..cut].to_owned(),
        None => text,
    }
}

/// Whether a character is of a script that writes many words in two
/// characters: Chinese, Japanese and Korean.
fn dense(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30ff}' // Hiragana, Katakana
        | '\u{3400}'..='\u{4dbf}' // CJK Extension A
        | '\u{4e00}'..='\u{9fff}' // CJK Unified Ideographs
        | '\u{ac00}'..='\u{d7af}' // Hangul syllables
        | '\u{f900}'..='\u{faff}' // CJK Compatibility Ideographs
        | '\u{20000}'..='\u{2fa1f}') // CJK Extensions B and on
}

/// A search, checked: [`normalize`]d, between [`MIN_QUERY_CHARS`] (two in
/// Chinese, Japanese or Korean) and [`MAX_QUERY_CHARS`] long.
pub fn query(input: &str) -> Result<String, ApiError> {
    let q = normalize(input);
    let n = q.chars().count();
    if n < MIN_QUERY_CHARS && !(n == 2 && q.chars().any(dense)) {
        return Err(ApiError::validation(
            "q",
            "Search for at least 3 characters (2 in Chinese, Japanese or Korean)",
        ));
    }
    if n > MAX_QUERY_CHARS {
        return Err(ApiError::validation(
            "q",
            "Search for at most 200 characters",
        ));
    }
    if q.chars().any(char::is_control) {
        return Err(ApiError::validation("q", "Remove control characters"));
    }
    Ok(q)
}

/// Whether an index of trigrams (SQLite's FTS5 `trigram` tokenizer) can
/// find `query`: it needs three characters at least. Two of Chinese,
/// Japanese or Korean are looked for row by row.
pub fn indexable(query: &str) -> bool {
    query.chars().count() >= 3
}

/// `query` as an FTS5 phrase, which with the `trigram` tokenizer matches
/// the documents that contain it, case and accents aside.
pub fn fts_phrase(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

/// `query` as a `LIKE` pattern (escaped with `\`), for searches too short
/// for [`fts_phrase`].
pub fn like_pattern(query: &str) -> String {
    let mut p = String::with_capacity(query.len() + 2);
    p.push('%');
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

/// A session's title and description as documents.
pub fn about(title: &str, description: Option<&str>, ts: i64) -> Vec<Doc> {
    let mut docs = vec![Doc {
        seq: 0,
        part: 0,
        kind: DocKind::Title,
        ts,
        author: None,
        text: doc_text(title),
    }];
    if let Some(d) = description.map(doc_text).filter(|d| !d.is_empty()) {
        docs.push(Doc {
            seq: 0,
            part: 1,
            kind: DocKind::Description,
            ts,
            author: None,
            text: d,
        });
    }
    docs
}

/// Whether an event holds something the index takes: a message with words,
/// or an agent's reply that says something (what [`docs_of`] makes
/// documents of).
pub fn searchable(body: &EventBody) -> bool {
    match body {
        EventBody::UserMessage(m) => !m.text.trim().is_empty(),
        EventBody::Claude(v) => reply_text(v).is_some(),
        _ => false,
    }
}

/// The searchable documents of a stored event (its `kind` and `data`, as
/// SessionDO keeps them): a message's text, or what an agent's reply says,
/// as the session page shows its Markdown ([`markdown_text`]). A
/// subagent's replies, which go to the agent, aren't the agent's.
pub fn docs_of(seq: i64, ts: i64, kind: &str, data: &Value) -> Vec<Doc> {
    let text = match kind {
        "user_message" => {
            let Ok(m) = serde_json::from_value::<UserMessage>(data.clone()) else {
                return Vec::new();
            };
            let text = doc_text(&m.text);
            if text.is_empty() {
                return Vec::new();
            }
            return vec![Doc {
                seq,
                part: 0,
                kind: DocKind::Message,
                ts,
                author: m.author,
                text,
            }];
        }
        "claude" => reply_text(data).map(|t| markdown_text(&t)),
        _ => None,
    };
    let Some(text) = text.map(|t| doc_text(&t)).filter(|t| !t.is_empty()) else {
        return Vec::new();
    };
    vec![Doc {
        seq,
        part: 0,
        kind: DocKind::Reply,
        ts,
        author: None,
        text,
    }]
}

/// The text of an agent's reply: the text blocks of an `assistant` message
/// of its own (not a subagent's), one paragraph each.
fn reply_text(v: &Value) -> Option<String> {
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    if v.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
        return None;
    }
    let parts: Vec<&str> = v
        .pointer("/message/content")?
        .as_array()?
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .filter(|t| !t.trim().is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Markdown's text as the session page shows it (web/src/lib/find.ts
/// `plainText`): what its blocks say, each on a line of its own, without
/// its syntax, link addresses or pictures, which show no text, so that a
/// search finds what the page's find bar does, and shows clean snippets.
pub fn markdown_text(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    // Inside pictures, whose text shows only for those the page doesn't load.
    let mut pictures: Vec<bool> = Vec::new();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(markdown, options) {
        let shown = pictures.iter().all(|&shows_text| shows_text);
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                pictures.push(!dest_url.to_ascii_lowercase().starts_with("https:"));
            }
            Event::End(TagEnd::Image) => {
                pictures.pop();
            }
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) if shown => {
                out.push_str(&t);
            }
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::HtmlBlock
                | TagEnd::Item
                | TagEnd::TableCell,
            ) => out.push('\n'),
            _ => {}
        }
    }
    out
}

/// A character as a search compares it: lowercase, and without its accent
/// for the Latin letters that have one, as SQLite's `trigram` tokenizer
/// does with `remove_diacritics 1`. Combining accents go (`None`).
fn fold(c: char) -> impl Iterator<Item = char> {
    c.to_lowercase().filter_map(|c| match c {
        '\u{300}'..='\u{36f}' => None,
        c => Some(base_letter(c)),
    })
}

/// The letter a Latin letter with a diacritic is written with.
fn base_letter(c: char) -> char {
    match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        c => c,
    }
}

/// `text` folded ([`fold`]), with the byte range in `text` of the
/// character each folded one comes from.
fn folded(text: &str) -> (Vec<char>, Vec<(usize, usize)>) {
    let mut chars = Vec::with_capacity(text.len());
    let mut at = Vec::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        for f in fold(c) {
            chars.push(f);
            at.push((i, i + c.len_utf8()));
        }
    }
    (chars, at)
}

/// Where `query` is in `text`, case and accents aside: the byte ranges of
/// its occurrences, which don't overlap.
pub fn find(text: &str, query: &str) -> Vec<(usize, usize)> {
    let needle: Vec<char> = query.chars().flat_map(fold).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let (hay, at) = folded(text);
    let mut found = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay.get(i..i + needle.len()) == Some(needle.as_slice()) {
            let start = at.get(i).map_or(0, |r| r.0);
            let end = at.get(i + needle.len() - 1).map_or(start, |r| r.1);
            // Folding never splits a character, so the next occurrence
            // starts after this one's last.
            found.push((start, end));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    found
}

/// What a result shows of a document: the words around where it matches,
/// cut at word boundaries with "…", and where in them it matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// The occurrences, as `[start, end)` in UTF-16 code units of `text`,
    /// as the web app slices it.
    pub ranges: Vec<[u32; 2]>,
}

/// A [`Snippet`] of `text` around the first place it contains `query`:
/// about [`SNIPPET_LEAD`] characters before it and [`SNIPPET_CHARS`] in
/// all. Text that doesn't contain it (as SQLite folded it otherwise)
/// shows from its start.
pub fn snippet(text: &str, query: &str) -> Snippet {
    let found = find(text, query);
    let (first, first_end) = found.first().copied().unwrap_or((0, 0));
    // From a word's start, about SNIPPET_LEAD characters before the match;
    // in text without spaces (Chinese, a long address), a little further.
    let mut start = first;
    for (n, (i, c)) in text[..first].char_indices().rev().enumerate() {
        if n >= SNIPPET_LEAD && c == ' ' {
            start = i + 1;
            break;
        }
        if n >= SNIPPET_LEAD + SNIPPET_SLACK {
            break;
        }
        start = i;
    }
    // To a word's end, about SNIPPET_CHARS characters after the start, and
    // past the whole first match, however long.
    let mut end = text.len();
    for (n, (i, c)) in text[start..].char_indices().enumerate() {
        let at = start + i;
        if at >= first_end
            && ((n >= SNIPPET_CHARS && c == ' ') || n >= SNIPPET_CHARS + SNIPPET_SLACK)
        {
            end = at;
            break;
        }
    }
    let before = start > 0;
    let after = end < text.len();
    let mut out = String::new();
    if before {
        out.push('…');
    }
    out.push_str(&text[start..end]);
    if after {
        out.push('…');
    }
    let offset = usize::from(before);
    let utf16 = |byte: usize| text[start..byte].encode_utf16().count() + offset;
    let ranges = found
        .into_iter()
        .filter(|&(s, e)| s >= start && e <= end)
        .map(|(s, e)| [utf16(s) as u32, utf16(e) as u32])
        .collect();
    Snippet { text: out, ranges }
}

/// A document as a result shows it.
pub fn to_match(
    kind: DocKind,
    seq: i64,
    ts: i64,
    author: Option<UserRef>,
    text: &str,
    query: &str,
) -> SearchMatch {
    let s = snippet(text, query);
    let about = matches!(kind, DocKind::Title | DocKind::Description);
    SearchMatch {
        kind: kind.to_match_kind(),
        seq: (!about).then_some(seq.max(0) as u64),
        ts: (!about).then_some(ts),
        author,
        text: s.text,
        ranges: s.ranges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::UserId;
    use serde_json::json;

    fn utf16_slice(s: &str, [a, b]: [u32; 2]) -> String {
        let units: Vec<u16> = s.encode_utf16().collect();
        String::from_utf16_lossy(&units[a as usize..b as usize])
    }

    #[test]
    fn whitespace_counts_as_one_space() {
        assert_eq!(
            normalize("  fix\tthe\n\nlogin   redirect "),
            "fix the login redirect"
        );
        assert_eq!(normalize("\n \t"), "");
    }

    #[test]
    fn searches_are_checked() {
        assert_eq!(
            query("  login \n redirect ").ok().as_deref(),
            Some("login redirect")
        );
        assert_eq!(query("abc").ok().as_deref(), Some("abc"));
        // Two characters find too much, but in Chinese, Japanese or Korean.
        assert_eq!(query("検索").ok().as_deref(), Some("検索"));
        assert_eq!(query("한국").ok().as_deref(), Some("한국"));
        assert_eq!(
            query("ab").err().and_then(|e| e.field).as_deref(),
            Some("q")
        );
        assert_eq!(
            query(" a ").err().and_then(|e| e.field).as_deref(),
            Some("q")
        );
        assert!(query(&"x".repeat(201)).is_err());
        assert!(query("a\u{7}b").is_err());
    }

    #[test]
    fn short_searches_go_row_by_row() {
        assert!(!indexable("ab"));
        assert!(!indexable("検索"));
        assert!(indexable("abc"));
        assert_eq!(like_pattern("50%_a\\b"), "%50\\%\\_a\\\\b%");
    }

    #[test]
    fn phrases_escape_quotes() {
        assert_eq!(fts_phrase("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(fts_phrase("auth.rs OR x*"), "\"auth.rs OR x*\"");
    }

    #[test]
    fn titles_and_descriptions_are_documents() {
        let docs = about("Fix the  login\nredirect", Some("It loops."), 5);
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].kind, DocKind::Title);
        assert_eq!(docs[0].text, "Fix the login redirect");
        assert_eq!((docs[1].seq, docs[1].part), (0, 1));
        assert_eq!(docs[1].kind, DocKind::Description);
        assert_eq!(about("x", Some("  "), 5).len(), 1);
        assert_eq!(about("x", None, 5).len(), 1);
    }

    #[test]
    fn messages_are_documents() {
        let data = json!({
            "id": "dlv_1",
            "author": {"id": "usr_1", "name": "Ana"},
            "text": "Please fix\nthe redirect",
            "source": "web",
        });
        let docs = docs_of(7, 100, "user_message", &data);
        assert_eq!(
            docs,
            vec![Doc {
                seq: 7,
                part: 0,
                kind: DocKind::Message,
                ts: 100,
                author: Some(UserRef {
                    id: UserId("usr_1".into()),
                    name: "Ana".into(),
                    avatar_url: None,
                }),
                text: "Please fix the redirect".into(),
            }]
        );
        // Only files: nothing to search.
        let files = json!({"id": "dlv_2", "text": "", "source": "web"});
        assert!(docs_of(8, 100, "user_message", &files).is_empty());
    }

    #[test]
    fn replies_are_the_agents_text() {
        let reply = json!({
            "type": "assistant",
            "message": {"id": "m1", "content": [
                {"type": "thinking", "thinking": "hmm"},
                {"type": "text", "text": "Fixed it."},
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"}},
                {"type": "text", "text": "Tests pass."},
            ]},
        });
        let docs = docs_of(9, 200, "claude", &reply);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].kind, DocKind::Reply);
        assert_eq!(docs[0].text, "Fixed it. Tests pass.");
        assert_eq!(docs[0].author, None);
        // A subagent's words go to the agent, not to people.
        let sub = json!({
            "type": "assistant",
            "parent_tool_use_id": "t0",
            "message": {"id": "m2", "content": [{"type": "text", "text": "Found 3 files."}]},
        });
        assert!(docs_of(10, 200, "claude", &sub).is_empty());
        // Tool calls and their results aren't searched.
        let call = json!({
            "type": "assistant",
            "message": {"id": "m3", "content": [{"type": "tool_use", "id": "t2", "name": "Read", "input": {}}]},
        });
        assert!(docs_of(11, 200, "claude", &call).is_empty());
        let result = json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "secret"}]}});
        assert!(docs_of(12, 200, "claude", &result).is_empty());
        assert!(docs_of(13, 200, "log", &json!({"text": "x"})).is_empty());
    }

    #[test]
    fn replies_are_searched_as_the_page_shows_them() {
        let reply = |text: &str| {
            let v = json!({
                "type": "assistant",
                "message": {"id": "m1", "content": [{"type": "text", "text": text}]},
            });
            docs_of(1, 1, "claude", &v)
                .into_iter()
                .next()
                .map(|d| d.text)
                .unwrap_or_default()
        };
        assert_eq!(
            reply(
                "Fixed **the login** redirect in `auth.rs`: see [the PR](https://github.com/acme/web/pull/42)."
            ),
            "Fixed the login redirect in auth.rs: see the PR."
        );
        assert_eq!(
            reply("# Done\n\n- one *thing*\n- two\n\n```sh\ncargo test -p fctl\n```"),
            "Done one thing two cargo test -p fctl"
        );
        assert_eq!(reply("| a | b |\n|---|---|\n| one | two |"), "a b one two");
        // A picture shows no text, but one the page doesn't load shows what it is.
        assert_eq!(
            reply("Look: ![the header](https://example.com/a.png) and ![a sketch](http://x/b.png)"),
            "Look: and a sketch"
        );
        assert_eq!(
            reply("A &amp; B <b>html</b> ~~gone~~ \\*kept\\*"),
            "A & B <b>html</b> gone *kept*"
        );
        assert_eq!(reply("- [x] done\n- [ ] to do"), "done to do");
        assert_eq!(
            markdown_text("line one  \nline two"),
            "line one\nline two\n"
        );
    }

    #[test]
    fn events_with_words_are_searchable() {
        let message = |text: &str| {
            EventBody::UserMessage(UserMessage {
                id: fctl_proto::DeliveryId("dlv_1".into()),
                author: None,
                text: text.into(),
                source: fctl_proto::event::MessageSource::Web,
                attachments: Vec::new(),
                follow_up: None,
                review: None,
                ticket: None,
            })
        };
        assert!(searchable(&message("Fix it")));
        assert!(!searchable(&message("  ")));
        let said = |content: Value| {
            EventBody::Claude(
                json!({"type": "assistant", "message": {"id": "m", "content": content}}),
            )
        };
        assert!(searchable(&said(
            json!([{"type": "text", "text": "Done."}])
        )));
        assert!(!searchable(&said(json!([{"type": "text", "text": " \n"}]))));
        assert!(!searchable(&said(
            json!([{"type": "tool_use", "id": "t", "name": "Bash", "input": {}}])
        )));
        assert!(!searchable(&EventBody::Claude(
            json!({"type": "result", "result": "Done."})
        )));
    }

    #[test]
    fn long_documents_keep_their_start() {
        let docs = about(&"word ".repeat(5_000), None, 0);
        assert_eq!(docs[0].text.chars().count(), MAX_DOC_CHARS);
    }

    #[test]
    fn finds_whatever_the_case_and_accents() {
        let text = "Le CAFÉ, le café et le cafe\u{301}.";
        let found = find(text, "cafe");
        assert_eq!(found.len(), 3);
        assert_eq!(&text[found[0].0..found[0].1], "CAFÉ");
        assert_eq!(&text[found[1].0..found[1].1], "café");
        assert_eq!(&text[found[2].0..found[2].1], "cafe");
        assert_eq!(find("résumé", "RÉSUMÉ").len(), 1);
        assert_eq!(find("aaaa", "aa").len(), 2);
        assert!(find("abc", "").is_empty());
        assert_eq!(find("検索機能を検索", "検索").len(), 2);
    }

    #[test]
    fn snippets_show_the_words_around_the_match() {
        let text = format!(
            "{} the login redirect loops when the cookie expires, {}",
            "Some words before the match that go on for quite a while here".repeat(2),
            "and more after it that go on and on until the end of the text. ".repeat(5),
        );
        let s = snippet(&text, "Login Redirect");
        assert!(s.text.starts_with('…'));
        assert!(s.text.ends_with('…'));
        assert!(s.text.chars().count() <= SNIPPET_CHARS + 2);
        assert_eq!(s.ranges.len(), 1);
        assert_eq!(utf16_slice(&s.text, s.ranges[0]), "login redirect");
        // Starts at a word.
        assert!(s.text[3..].starts_with(|c: char| c.is_alphanumeric()));
        let lead = utf16_slice(&s.text, [1, s.ranges[0][0]]);
        assert!(lead.chars().count() >= SNIPPET_LEAD);
    }

    #[test]
    fn text_without_spaces_is_cut_anyway() {
        let text = format!("{}検索機能{}", "日本語".repeat(100), "テキスト".repeat(100));
        let s = snippet(&text, "検索");
        let n = s.text.chars().count();
        assert!(n <= SNIPPET_CHARS + SNIPPET_SLACK + 2, "{n} characters");
        assert_eq!(utf16_slice(&s.text, s.ranges[0]), "検索");
        let lead = utf16_slice(&s.text, [1, s.ranges[0][0]]).chars().count();
        assert_eq!(lead, SNIPPET_LEAD + SNIPPET_SLACK);
        let url = format!(
            "https://example.com/{}/needle/{}",
            "a".repeat(300),
            "b".repeat(300)
        );
        let s = snippet(&url, "needle");
        assert!(s.text.chars().count() <= SNIPPET_CHARS + SNIPPET_SLACK + 2);
        assert_eq!(utf16_slice(&s.text, s.ranges[0]), "needle");
    }

    #[test]
    fn long_matches_show_whole() {
        let query = "x".repeat(MAX_QUERY_CHARS);
        let text = format!("{} {query} and the rest", "word ".repeat(40));
        let s = snippet(&text, &query);
        assert_eq!(utf16_slice(&s.text, s.ranges[0]), query);
    }

    #[test]
    fn short_texts_show_whole() {
        let s = snippet("Fix the login redirect", "LOGIN");
        assert_eq!(s.text, "Fix the login redirect");
        assert_eq!(s.ranges, vec![[8, 13]]);
        let s = snippet("café, café", "cafe");
        assert_eq!(s.ranges, vec![[0, 4], [6, 10]]);
    }

    #[test]
    fn ranges_count_utf16() {
        let s = snippet("😀 then the bug", "bug");
        assert_eq!(utf16_slice(&s.text, s.ranges[0]), "bug");
        assert_eq!(s.ranges, vec![[12, 15]]);
    }

    #[test]
    fn text_without_the_match_shows_from_its_start() {
        let s = snippet("Nothing here matches", "zebra");
        assert_eq!(s.text, "Nothing here matches");
        assert!(s.ranges.is_empty());
    }

    #[test]
    fn results_say_where_they_matched() {
        let m = to_match(DocKind::Title, 0, 1, None, "Fix the redirect", "redirect");
        assert_eq!(m.kind, SearchMatchKind::Title);
        assert_eq!((m.seq, m.ts), (None, None));
        let m = to_match(
            DocKind::Reply,
            42,
            7,
            None,
            "Fixed the redirect",
            "redirect",
        );
        assert_eq!((m.seq, m.ts), (Some(42), Some(7)));
        assert_eq!(m.ranges, vec![[10, 18]]);
    }
}
