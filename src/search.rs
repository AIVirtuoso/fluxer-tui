//! Turning what the reader typed into a search request.
//!
//! The web client puts its filters in a row of pills built by clicking.
//! With a keyboard the same filters are worth typing, so the query takes
//! `from:`, `has:` and `pinned:` prefixes the way a mail client or a code
//! host does, and anything in double quotes has to appear together.
//! Everything else is ordinary words.

use crate::api::types::MessageSearchRequest;

/// The scopes the server offers, in the order the overlay walks them.
/// `current` means the channel or community the reader is in, which is
/// why it needs the context ids sent with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    Channel,
    Guild,
    Everything,
}

impl SearchScope {
    pub const ALL: [Self; 3] = [Self::Channel, Self::Guild, Self::Everything];

    pub fn label(self) -> &'static str {
        match self {
            Self::Channel => "this channel",
            Self::Guild => "this community",
            Self::Everything => "everywhere",
        }
    }

    /// What the server calls it. A channel search is `current` with a
    /// context channel; a community search is `current` with a context
    /// guild and no channel.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Channel | Self::Guild => "current",
            Self::Everything => "all",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// What a query said, before the names in it are turned into ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedQuery {
    /// The ordinary words.
    pub words: Vec<String>,
    /// Anything that was in double quotes.
    pub phrases: Vec<String>,
    /// The names after `from:`, still names.
    pub authors: Vec<String>,
    /// The kinds after `has:`, already checked against what the server
    /// knows; an unknown one is left as a word instead.
    pub has: Vec<String>,
    pub pinned: Option<bool>,
}

/// The values `has:` takes. Anything else is not a filter and is
/// searched for as text, which is friendlier than refusing the query.
const HAS_KINDS: [&str; 5] = ["image", "sound", "video", "file", "embed"];

/// Split a query into its parts. Quotes group words into a phrase; a
/// `from:`, `has:` or `pinned:` prefix takes the word after the colon.
pub fn parse(query: &str) -> ParsedQuery {
    let mut out = ParsedQuery::default();
    for token in tokenise(query) {
        match token {
            Token::Phrase(text) => {
                if !text.trim().is_empty() {
                    out.phrases.push(text);
                }
            }
            Token::Word(word) => {
                let lower = word.to_lowercase();
                if let Some(name) = strip_prefix_ci(&word, "from:") {
                    if !name.is_empty() {
                        out.authors.push(name.to_string());
                        continue;
                    }
                } else if let Some(kind) = strip_prefix_ci(&word, "has:") {
                    let kind = kind.to_lowercase();
                    if HAS_KINDS.contains(&kind.as_str()) {
                        out.has.push(kind);
                        continue;
                    }
                } else if let Some(value) = strip_prefix_ci(&word, "pinned:") {
                    match value.to_lowercase().as_str() {
                        "true" | "yes" => {
                            out.pinned = Some(true);
                            continue;
                        }
                        "false" | "no" => {
                            out.pinned = Some(false);
                            continue;
                        }
                        _ => {}
                    }
                }
                let _ = lower;
                out.words.push(word);
            }
        }
    }
    out
}

enum Token {
    Word(String),
    Phrase(String),
}

/// Words, with anything inside double quotes kept whole. An unclosed
/// quote runs to the end rather than being thrown away, so the query
/// still works while it is being typed.
fn tokenise(query: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in query.chars() {
        match ch {
            '"' => {
                if in_quotes {
                    out.push(Token::Phrase(std::mem::take(&mut current)));
                    in_quotes = false;
                } else {
                    if !current.trim().is_empty() {
                        out.push(Token::Word(std::mem::take(&mut current)));
                    }
                    current.clear();
                    in_quotes = true;
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    out.push(Token::Word(std::mem::take(&mut current)));
                }
            }
            c => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(if in_quotes {
            Token::Phrase(current)
        } else {
            Token::Word(current)
        });
    }
    out
}

fn strip_prefix_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if value.len() < prefix.len() {
        return None;
    }
    value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .and_then(|_| value.get(prefix.len()..))
}

/// Build the request. `authors` are the ids the caller resolved the
/// `from:` names to; a name that matched nobody is dropped by the caller
/// rather than silently searched for as text, so the reader is told.
pub fn build_request(
    parsed: &ParsedQuery,
    author_ids: Vec<String>,
    scope: SearchScope,
    context_channel_id: Option<String>,
    context_guild_id: Option<String>,
    page: u32,
) -> MessageSearchRequest {
    let content = parsed.words.join(" ");
    MessageSearchRequest {
        content: (!content.trim().is_empty()).then_some(content),
        exact_phrases: parsed.phrases.clone(),
        author_id: author_ids,
        has: parsed.has.clone(),
        pinned: parsed.pinned,
        scope: Some(scope.wire().to_string()),
        // a channel search names the channel; a community search names
        // only the community, so every channel in it is in scope
        context_channel_id: match scope {
            SearchScope::Channel => context_channel_id,
            _ => None,
        },
        context_guild_id: match scope {
            SearchScope::Channel | SearchScope::Guild => context_guild_id,
            SearchScope::Everything => None,
        },
        channel_ids: Vec::new(),
        page: page.max(1),
        hits_per_page: 25,
        sort_by: "timestamp".to_string(),
    }
}

/// Whether a query says anything the server could act on. An empty one,
/// or one that is only a `pinned:` with no words, is still a search — but
/// one with nothing at all in it is not worth sending.
pub fn is_searchable(parsed: &ParsedQuery) -> bool {
    !parsed.words.is_empty()
        || !parsed.phrases.is_empty()
        || !parsed.authors.is_empty()
        || !parsed.has.is_empty()
        || parsed.pinned.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_are_the_content() {
        let q = parse("  sixel  transparency ");
        assert_eq!(q.words, vec!["sixel", "transparency"]);
        assert!(q.phrases.is_empty());
        assert!(is_searchable(&q));
    }

    #[test]
    fn quotes_make_a_phrase_that_has_to_appear_together() {
        let q = parse(r#"before "no more combinations" after"#);
        assert_eq!(q.words, vec!["before", "after"]);
        assert_eq!(q.phrases, vec!["no more combinations"]);
    }

    #[test]
    fn an_unclosed_quote_still_searches_for_what_is_typed_so_far() {
        let q = parse(r#"look "half a phr"#);
        assert_eq!(q.words, vec!["look"]);
        assert_eq!(q.phrases, vec!["half a phr"]);
    }

    #[test]
    fn from_has_and_pinned_are_filters_and_the_case_does_not_matter() {
        let q = parse("From:ada HAS:image pinned:TRUE hello");
        assert_eq!(q.authors, vec!["ada"]);
        assert_eq!(q.has, vec!["image"]);
        assert_eq!(q.pinned, Some(true));
        assert_eq!(q.words, vec!["hello"]);
    }

    #[test]
    fn an_unknown_filter_value_is_searched_for_as_text_rather_than_refused() {
        // "has:" only knows five kinds; anything else is somebody typing
        // about a thing, not asking for a filter
        let q = parse("has:sausages pinned:maybe");
        assert!(q.has.is_empty());
        assert_eq!(q.pinned, None);
        assert_eq!(q.words, vec!["has:sausages", "pinned:maybe"]);
    }

    #[test]
    fn a_bare_prefix_is_a_word_not_an_empty_filter() {
        let q = parse("from: has:");
        assert!(q.authors.is_empty());
        assert!(q.has.is_empty());
        assert_eq!(q.words, vec!["from:", "has:"]);
    }

    #[test]
    fn an_empty_query_is_not_worth_sending_but_a_filter_alone_is() {
        assert!(!is_searchable(&parse("   ")));
        assert!(is_searchable(&parse("has:image")));
        assert!(is_searchable(&parse("pinned:true")));
    }

    #[test]
    fn a_channel_search_names_the_channel_and_a_community_one_does_not() {
        let q = parse("hello");
        let channel = build_request(
            &q,
            Vec::new(),
            SearchScope::Channel,
            Some("c1".to_string()),
            Some("g1".to_string()),
            1,
        );
        assert_eq!(channel.scope.as_deref(), Some("current"));
        assert_eq!(channel.context_channel_id.as_deref(), Some("c1"));
        assert_eq!(channel.context_guild_id.as_deref(), Some("g1"));

        let guild = build_request(
            &q,
            Vec::new(),
            SearchScope::Guild,
            Some("c1".to_string()),
            Some("g1".to_string()),
            1,
        );
        assert_eq!(guild.context_channel_id, None);
        assert_eq!(guild.context_guild_id.as_deref(), Some("g1"));

        let all = build_request(
            &q,
            Vec::new(),
            SearchScope::Everything,
            Some("c1".to_string()),
            Some("g1".to_string()),
            1,
        );
        assert_eq!(all.scope.as_deref(), Some("all"));
        assert_eq!(all.context_channel_id, None);
        assert_eq!(all.context_guild_id, None);
    }

    #[test]
    fn the_scopes_go_round_both_ways() {
        assert_eq!(SearchScope::Channel.next(), SearchScope::Guild);
        assert_eq!(SearchScope::Everything.next(), SearchScope::Channel);
        assert_eq!(SearchScope::Channel.previous(), SearchScope::Everything);
    }

    #[test]
    fn a_page_is_never_zero_since_the_server_counts_from_one() {
        let request = build_request(
            &parse("x"),
            Vec::new(),
            SearchScope::Everything,
            None,
            None,
            0,
        );
        assert_eq!(request.page, 1);
    }
}
