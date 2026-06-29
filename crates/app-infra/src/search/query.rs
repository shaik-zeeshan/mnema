//! Search query parsing, tokenization, and refinement normalization.
//!
//! Pure-move extraction from `search/mod.rs`: refinement normalization plus the
//! quote-aware query parser, tokenizer, and Body Match Operator -> FTS5
//! translation. No logic changes.

use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

use crate::{AppInfraError, AudioSegmentSourceKind, Result};

use super::dates::{
    end_of_day_rfc3339, local_today_date, open_lower_bound_rfc3339, open_upper_bound_rfc3339,
    resolve_day_or_period, resolve_point_date, start_of_day_rfc3339,
};
use super::types::{
    normalize_app_name_for_search, NormalizedAppRefinement, NormalizedDateRange,
    NormalizedSearchRefinements, SearchAppRefinement, SearchAppRefinementKind,
    SearchCaptureRefinements, SearchDateRangeRefinement, SearchParseError,
};

/// Outcome of normalizing refinements: either the normalized form, or a set of
/// in-band parse errors that should suppress results without throwing.
type NormalizationOutcome = std::result::Result<NormalizedSearchRefinements, Vec<SearchParseError>>;

/// Builds a [`SearchParseError`] with a whole-query span. Used for refinement
/// problems that have no narrower token origin (for example the app/source
/// conflict, which spans the combination rather than one token).
fn whole_query_parse_error(kind: &str, message: impl Into<String>) -> SearchParseError {
    SearchParseError {
        kind: kind.to_string(),
        message: message.into(),
        start: 0,
        end: 0,
        token: String::new(),
    }
}

pub(super) fn normalize_search_refinements(
    refinements: Option<SearchCaptureRefinements>,
) -> Result<NormalizationOutcome> {
    let refinements = refinements.unwrap_or_default();
    let screen_source = refinements.screen_source;
    let mut errors: Vec<SearchParseError> = Vec::new();

    if !refinements.apps.is_empty() && !refinements.audio_sources.is_empty() {
        errors.push(whole_query_parse_error(
            "app_source_conflict",
            "app and source operators cannot be combined: app narrows screen results while source narrows audio results",
        ));
    }

    if screen_source && !refinements.audio_sources.is_empty() {
        errors.push(whole_query_parse_error(
            "screen_audio_source_conflict",
            "source:screen cannot be combined with source:mic or source:system: screen narrows captured frames while those narrow audio",
        ));
    }

    let date_range = match refinements.date_range {
        Some(range) => match normalize_date_range_refinement(range) {
            Ok(resolved) => Some(resolved),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => None,
    };

    let mut normalized_apps = Vec::new();
    let mut applied_apps = Vec::new();
    for app in refinements.apps {
        match normalize_app_refinement(app) {
            Ok((normalized, applied)) => {
                if !applied_apps.contains(&applied) {
                    normalized_apps.push(normalized);
                    applied_apps.push(applied);
                }
            }
            Err(error) => errors.push(error),
        }
    }

    let window_title = match refinements.window_title {
        Some(value) => {
            let value = value.trim().to_string();
            if value.is_empty() {
                errors.push(whole_query_parse_error(
                    "empty_value",
                    "windowTitle must be non-empty",
                ));
                None
            } else {
                Some(value)
            }
        }
        None => None,
    };

    let mut audio_sources = Vec::new();
    for source in refinements.audio_sources {
        if !audio_sources.contains(&source) {
            audio_sources.push(source);
        }
    }

    if !errors.is_empty() {
        return Ok(Err(errors));
    }

    Ok(Ok(NormalizedSearchRefinements {
        date_range: date_range
            .as_ref()
            .map(|(normalized, _)| normalized.clone()),
        apps: normalized_apps,
        window_title: window_title.clone(),
        audio_sources: audio_sources.clone(),
        screen_source,
        applied: SearchCaptureRefinements {
            date_range: date_range.map(|(_, applied)| applied),
            apps: applied_apps,
            window_title,
            audio_sources,
            screen_source,
        },
    }))
}

fn normalize_date_range_refinement(
    range: SearchDateRangeRefinement,
) -> std::result::Result<(NormalizedDateRange, SearchDateRangeRefinement), SearchParseError> {
    let start = OffsetDateTime::parse(range.start_at.trim(), &Rfc3339).map_err(|_| {
        whole_query_parse_error(
            "bad_date",
            "date range start must be a valid RFC3339 timestamp",
        )
    })?;
    let end = OffsetDateTime::parse(range.end_at.trim(), &Rfc3339).map_err(|_| {
        whole_query_parse_error(
            "bad_date",
            "date range end must be a valid RFC3339 timestamp",
        )
    })?;
    if start > end {
        return Err(whole_query_parse_error(
            "bad_date",
            "date range start must be before or equal to date range end",
        ));
    }
    let start_at = format_rfc3339_for_search(start)
        .map_err(|error| whole_query_parse_error("bad_date", error.to_string()))?;
    let end_at = format_rfc3339_for_search(end)
        .map_err(|error| whole_query_parse_error("bad_date", error.to_string()))?;
    Ok((
        NormalizedDateRange {
            start_at: start_at.clone(),
            end_at: end_at.clone(),
        },
        SearchDateRangeRefinement {
            start_at,
            end_at,
            origin: range.origin,
        },
    ))
}

fn normalize_app_refinement(
    app: SearchAppRefinement,
) -> std::result::Result<(NormalizedAppRefinement, SearchAppRefinement), SearchParseError> {
    let value = app.value.trim().to_string();
    let display_name = app.display_name.trim().to_string();
    if value.is_empty() {
        return Err(whole_query_parse_error(
            "empty_value",
            "app value must be non-empty",
        ));
    }
    let normalized = match app.kind {
        SearchAppRefinementKind::Any => NormalizedAppRefinement::Any {
            value: value.clone(),
            search_key: normalize_app_name_for_search(&value).ok_or_else(|| {
                whole_query_parse_error("empty_value", "app value must be non-empty")
            })?,
        },
        SearchAppRefinementKind::BundleId => NormalizedAppRefinement::BundleId {
            value: value.clone(),
        },
        SearchAppRefinementKind::AppName => NormalizedAppRefinement::AppName {
            search_key: normalize_app_name_for_search(&value).ok_or_else(|| {
                whole_query_parse_error("empty_value", "app value must be non-empty")
            })?,
        },
    };
    Ok((
        normalized,
        SearchAppRefinement {
            kind: app.kind,
            value,
            display_name: if display_name.is_empty() {
                app.value.trim().to_string()
            } else {
                display_name
            },
        },
    ))
}

pub(super) fn format_rfc3339_for_search(value: OffsetDateTime) -> Result<String> {
    value
        .to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|error| AppInfraError::InvalidSearchRequest(error.to_string()))
}

pub(super) fn normalize_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fts_query_for_plain_text(query: &str) -> String {
    let terms = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let mut searchable_terms = terms
        .iter()
        .copied()
        .filter(|term| term.chars().count() >= 2)
        .collect::<Vec<_>>();
    if searchable_terms.is_empty() && query.chars().count() >= 2 {
        searchable_terms = terms;
    }
    searchable_terms
        .into_iter()
        .map(|term| fts_quote_phrase_term(term))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Wraps a single token as a safe FTS5 quoted phrase term, doubling embedded
/// quotes. This is the canonical escaping used everywhere body text reaches
/// MATCH so raw user input never reaches FTS5 unquoted.
fn fts_quote_phrase_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

// === Search Query Syntax (ADR 0019) ===
//
// `parse_search_query` is the backend-canonical parser. It is quote-aware,
// recognizes only the known field operators, extracts them into refinements,
// translates the residual body operators into a safe FTS5 expression, and
// returns any strict validation problems as in-band parse errors with
// character (Unicode scalar) spans into the original raw query.

/// The known field operator keys. Any other `key:value` token stays literal
/// body text so URL, code, and `error:404`-style searches keep working.
const FIELD_OPERATOR_KEYS: &[&str] = &["app", "source", "after", "before", "date"];

/// Result of parsing a raw search query into refinements + residual body FTS.
#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedQuery {
    /// The safe FTS5 match expression derived from the residual body.
    pub(crate) fts_body: String,
    /// `app:` operators, extracted as `Any`-kind app refinements.
    pub(crate) apps: Vec<SearchAppRefinement>,
    /// `source:` operators, extracted as audio source kinds.
    pub(crate) audio_sources: Vec<AudioSegmentSourceKind>,
    /// `source:screen` operator, restricting results to captured frames.
    pub(crate) screen_source: bool,
    /// `after:`/`before:`/`date:` operators resolved to a single date range.
    pub(crate) date_range: Option<SearchDateRangeRefinement>,
    /// The plain residual body text (operators stripped) for display and FTS.
    pub(crate) residual_query: String,
    /// Strict validation problems found during parsing.
    pub(crate) errors: Vec<SearchParseError>,
}

/// One tokenizer token, carrying the original character span for error
/// reporting and whether the token (or its value) was quoted.
#[derive(Debug, Clone)]
struct QueryToken {
    /// The token text with surrounding quotes removed.
    text: String,
    /// True when the token text was wrapped in double quotes.
    quoted: bool,
    /// Character (Unicode scalar) start offset into the original raw query.
    start: u32,
    /// Character (Unicode scalar) end offset (exclusive) into the raw query.
    end: u32,
    /// The raw token slice exactly as typed (including quotes), for echoing.
    raw: String,
}

/// Outcome of quote-aware tokenization.
struct Tokenized {
    tokens: Vec<QueryToken>,
    /// Present when a quote was opened but never closed.
    unbalanced_quote: Option<SearchParseError>,
}

/// Quote-aware tokenizer. Splits on unquoted whitespace, keeps quoted runs
/// (including embedded whitespace) as a single token, and tracks character
/// spans into the original query.
fn tokenize_query(raw: &str) -> Tokenized {
    let chars: Vec<char> = raw.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0_usize;
    let len = chars.len();

    while index < len {
        // Skip unquoted whitespace between tokens.
        while index < len && chars[index].is_whitespace() {
            index += 1;
        }
        if index >= len {
            break;
        }

        let token_start = index;
        let mut text = String::new();
        let mut had_quote = false;
        let mut had_unquoted = false;

        while index < len {
            let ch = chars[index];
            if ch == '"' {
                had_quote = true;
                // Toggle quote mode. Any whitespace inside quotes is literal.
                let mut in_quote = true;
                index += 1;
                while index < len {
                    let inner = chars[index];
                    if inner == '"' {
                        // A doubled `""` inside the run is an escaped literal
                        // quote: consume both and keep one `"` in the phrase
                        // rather than closing and reopening the quoted run.
                        if index + 1 < len && chars[index + 1] == '"' {
                            text.push('"');
                            index += 2;
                            continue;
                        }
                        in_quote = false;
                        index += 1;
                        break;
                    }
                    text.push(inner);
                    index += 1;
                }
                if in_quote {
                    // Unterminated quote: report against the rest of the query.
                    let token_end = len as u32;
                    return Tokenized {
                        tokens,
                        unbalanced_quote: Some(SearchParseError {
                            kind: "unbalanced_quote".to_string(),
                            message: "a quoted phrase is missing its closing quote".to_string(),
                            start: token_start as u32,
                            end: token_end,
                            token: chars[token_start..len].iter().collect(),
                        }),
                    };
                }
            } else if ch.is_whitespace() {
                break;
            } else {
                text.push(ch);
                had_unquoted = true;
                index += 1;
            }
        }

        let token_end = index as u32;
        // A token is "quoted" (a literal body phrase) only when it was a pure
        // quoted run with no characters outside the quotes. A mixed token such
        // as `app:"Google Chrome"` keeps its key visible so it can still be
        // recognized as a field operator rather than a literal body phrase.
        let quoted = had_quote && !had_unquoted;
        let raw: String = chars[token_start..index as usize].iter().collect();
        tokens.push(QueryToken {
            text,
            quoted,
            start: token_start as u32,
            end: token_end,
            raw,
        });
    }

    Tokenized {
        tokens,
        unbalanced_quote: None,
    }
}

/// Splits a token into a `(key, value)` field-operator pair when it looks like
/// `key:value` with a non-empty alphanumeric key. The split happens on the
/// first unquoted colon in the raw token, so `app:"Google Chrome"` and
/// `error:404` are both detected as `key:value` shapes (recognition of known
/// keys happens separately).
fn split_field_operator(token: &QueryToken) -> Option<(String, String, bool)> {
    // Fully-quoted tokens are always literal body phrases, never operators.
    if token.quoted {
        return None;
    }
    let raw = &token.raw;
    let colon = raw.find(':')?;
    let key = &raw[..colon];
    if key.is_empty() || !key.chars().all(|ch| ch.is_alphanumeric()) {
        return None;
    }
    let value_raw = &raw[colon + 1..];
    // Strip surrounding quotes on the value (e.g. app:"Google Chrome").
    let (value, value_quoted) =
        if value_raw.starts_with('"') && value_raw.ends_with('"') && value_raw.chars().count() >= 2
        {
            (
                value_raw[1..value_raw.len() - 1].replace("\"\"", "\""),
                true,
            )
        } else {
            (value_raw.to_string(), false)
        };
    Some((key.to_string(), value, value_quoted))
}

/// The operator-stripped residual body of a raw search query — the text the
/// meaning-vector embed should use (so `app:`/`before:`/quoted operators don't
/// pollute the **Semantic Search Vector**). Mirrors what FTS ranks on.
///
/// app-infra takes no embedding-runtime dependency, so it cannot embed the query
/// itself; the desktop layer embeds, and this exposes the residual it should feed
/// the embedder. An all-operators query yields an empty residual, which the caller
/// treats as "no meaning vector" (keyword-only).
pub fn semantic_search_residual_query(raw: &str) -> String {
    parse_search_query(raw).residual_query
}

/// Parses a raw query into refinements and a safe FTS body. See module comment.
pub(crate) fn parse_search_query(raw: &str) -> ParsedQuery {
    let tokenized = tokenize_query(raw);
    let mut parsed = ParsedQuery::default();
    if let Some(error) = tokenized.unbalanced_quote {
        parsed.errors.push(error);
    }

    let local_today = local_today_date();
    let mut residual_tokens: Vec<QueryToken> = Vec::new();

    for token in tokenized.tokens {
        if let Some((key, value, value_quoted)) = split_field_operator(&token) {
            let lower_key = key.to_lowercase();
            if FIELD_OPERATOR_KEYS.contains(&lower_key.as_str()) {
                apply_field_operator(
                    &mut parsed,
                    &lower_key,
                    &value,
                    value_quoted,
                    &token,
                    local_today,
                );
                continue;
            }
        }
        // Not a known field operator: stays literal body text.
        residual_tokens.push(token);
    }

    parsed.residual_query = residual_tokens
        .iter()
        .map(|token| token.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    parsed.fts_body = fts_body_for_tokens(&residual_tokens, &mut parsed.errors);

    parsed
}

/// Applies one recognized field operator to the parsed refinements.
fn apply_field_operator(
    parsed: &mut ParsedQuery,
    key: &str,
    value: &str,
    _value_quoted: bool,
    token: &QueryToken,
    local_today: time::Date,
) {
    let trimmed = value.trim();
    match key {
        "app" => {
            if trimmed.is_empty() {
                parsed.errors.push(token_parse_error(
                    token,
                    "empty_value",
                    "app: needs an application name or bundle id",
                ));
                return;
            }
            let app = SearchAppRefinement {
                kind: SearchAppRefinementKind::Any,
                value: trimmed.to_string(),
                display_name: trimmed.to_string(),
            };
            if !parsed.apps.contains(&app) {
                parsed.apps.push(app);
            }
        }
        "source" => match trimmed.to_lowercase().as_str() {
            "mic" | "microphone" => {
                if !parsed
                    .audio_sources
                    .contains(&AudioSegmentSourceKind::Microphone)
                {
                    parsed.audio_sources.push(AudioSegmentSourceKind::Microphone);
                }
            }
            "system" | "system_audio" => {
                if !parsed
                    .audio_sources
                    .contains(&AudioSegmentSourceKind::SystemAudio)
                {
                    parsed
                        .audio_sources
                        .push(AudioSegmentSourceKind::SystemAudio);
                }
            }
            "screen" => parsed.screen_source = true,
            _ => parsed.errors.push(token_parse_error(
                token,
                "unknown_source",
                "source: must be mic, system, or screen",
            )),
        },
        "after" => match resolve_point_date(trimmed, local_today) {
            Some(date) => set_date_bound(parsed, Some(start_of_day_rfc3339(date)), None),
            None => parsed.errors.push(token_parse_error(
                token,
                "bad_date",
                "after: needs a date (YYYY-MM-DD) or relative point (today, yesterday, Nd, Nh)",
            )),
        },
        "before" => match resolve_point_date(trimmed, local_today) {
            Some(date) => set_date_bound(parsed, None, Some(end_of_day_rfc3339(date))),
            None => parsed.errors.push(token_parse_error(
                token,
                "bad_date",
                "before: needs a date (YYYY-MM-DD) or relative point (today, yesterday, Nd, Nh)",
            )),
        },
        "date" => match resolve_day_or_period(trimmed, local_today) {
            Some((start_date, end_date)) => set_date_bound(
                parsed,
                Some(start_of_day_rfc3339(start_date)),
                Some(end_of_day_rfc3339(end_date)),
            ),
            None => parsed.errors.push(token_parse_error(
                token,
                "bad_date",
                "date: needs a day or period (today, yesterday, last-week, this-week, last-month, this-month, or YYYY-MM-DD)",
            )),
        },
        _ => {}
    }
}

/// Writes one or both bounds into the single date range slot, last-write-wins
/// per bound. A one-sided write leaves the other bound at the wide-open
/// sentinel so the range stays half-open at day granularity.
fn set_date_bound(parsed: &mut ParsedQuery, start: Option<String>, end: Option<String>) {
    let existing = parsed.date_range.take();
    let mut start_at = existing
        .as_ref()
        .map(|range| range.start_at.clone())
        .unwrap_or_else(open_lower_bound_rfc3339);
    let mut end_at = existing
        .as_ref()
        .map(|range| range.end_at.clone())
        .unwrap_or_else(open_upper_bound_rfc3339);
    if let Some(start) = start {
        start_at = start;
    }
    if let Some(end) = end {
        end_at = end;
    }
    parsed.date_range = Some(SearchDateRangeRefinement {
        start_at,
        end_at,
        origin: None,
    });
}

fn token_parse_error(token: &QueryToken, kind: &str, message: &str) -> SearchParseError {
    SearchParseError {
        kind: kind.to_string(),
        message: message.to_string(),
        start: token.start,
        end: token.end,
        token: token.raw.clone(),
    }
}

// --- Body Match Operator → FTS5 translation ---

/// One residual body element after operator interpretation.
#[derive(Debug, Clone)]
enum BodyTerm {
    /// A positive matchable element (already a safe FTS5 fragment).
    Positive(String),
    /// A negated element (`-term`); requires a positive sibling in its AND group.
    Negative(String),
}

/// Translates residual tokens into a safe FTS5 expression. Body operators:
/// quoted phrase, `-term` exclusion (FTS5 NOT), `OR` (uppercase only),
/// `term*` prefix (>=2 leading chars), and implicit AND between terms.
///
/// When the residual contains no body operators at all, this delegates to the
/// exact plain-text path so operator-free queries behave identically to before.
fn fts_body_for_tokens(tokens: &[QueryToken], errors: &mut Vec<SearchParseError>) -> String {
    if tokens.is_empty() {
        return String::new();
    }

    let has_body_operator = tokens.iter().any(|token| {
        token.quoted
            || token.text == "OR"
            || token.text.starts_with('-')
            || token.text.ends_with('*')
    });

    if !has_body_operator {
        let joined = tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        return fts_query_for_plain_text(&normalize_query(&joined));
    }

    // Split into OR-groups on bare uppercase `OR` tokens; within each group,
    // terms are implicitly ANDed (AND binds tighter than OR in FTS5).
    let mut groups: Vec<Vec<&QueryToken>> = vec![Vec::new()];
    let mut or_tokens: Vec<&QueryToken> = Vec::new();
    for token in tokens {
        if !token.quoted && token.text == "OR" {
            or_tokens.push(token);
            groups.push(Vec::new());
            continue;
        }
        groups
            .last_mut()
            .expect("there is always at least one group")
            .push(token);
    }

    // A dangling `OR` (leading, trailing, or doubled) leaves an empty AND-group.
    // ADR 0019 mandates strict validation of malformed Body Match Operators, so
    // reject it as an in-band parse error instead of silently rewriting
    // `foo OR` into `foo`. Attribute the error to the OR adjacent to the gap.
    if let Some(empty_index) = groups.iter().position(|group| group.is_empty()) {
        let or_index = empty_index
            .saturating_sub(1)
            .min(or_tokens.len().saturating_sub(1));
        if let Some(token) = or_tokens.get(or_index) {
            errors.push(token_parse_error(
                token,
                "dangling_or",
                "OR needs a search term on both sides",
            ));
        }
        return String::new();
    }

    let mut rendered_groups: Vec<String> = Vec::new();
    for group in groups {
        if let Some(rendered) = fts_and_group(&group, errors) {
            if !rendered.is_empty() {
                rendered_groups.push(rendered);
            }
        }
    }

    rendered_groups
        .into_iter()
        .map(|group| format!("({group})"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Renders one implicit-AND group of tokens into an FTS5 fragment. Returns
/// `None` (and records an error) for pure-negation groups.
fn fts_and_group(tokens: &[&QueryToken], errors: &mut Vec<SearchParseError>) -> Option<String> {
    let mut body_terms: Vec<BodyTerm> = Vec::new();
    let mut positive_count = 0_usize;
    let mut negative_origin: Option<&QueryToken> = None;

    for token in tokens {
        if token.quoted {
            // Quoted phrase forces literal matching of the whole phrase.
            if token.text.trim().is_empty() {
                continue;
            }
            body_terms.push(BodyTerm::Positive(fts_quote_phrase_term(&token.text)));
            positive_count += 1;
            continue;
        }

        let text = token.text.as_str();
        if let Some(stripped) = text.strip_prefix('-') {
            if let Some(fragment) = fts_fragment_for_word(stripped) {
                body_terms.push(BodyTerm::Negative(fragment));
                if negative_origin.is_none() {
                    negative_origin = Some(token);
                }
            }
            continue;
        }

        if let Some(fragment) = fts_fragment_for_word(text) {
            body_terms.push(BodyTerm::Positive(fragment));
            positive_count += 1;
        }
    }

    if positive_count == 0 {
        if let Some(token) = negative_origin {
            errors.push(token_parse_error(
                token,
                "pure_negation",
                "an exclusion (-term) needs at least one positive term to match",
            ));
        }
        return None;
    }

    let positives = body_terms
        .iter()
        .filter_map(|term| match term {
            BodyTerm::Positive(fragment) => Some(fragment.clone()),
            BodyTerm::Negative(_) => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    let negatives = body_terms
        .iter()
        .filter_map(|term| match term {
            BodyTerm::Negative(fragment) => Some(fragment.clone()),
            BodyTerm::Positive(_) => None,
        })
        .collect::<Vec<_>>();

    if negatives.is_empty() {
        Some(positives)
    } else {
        Some(format!("{positives} NOT {}", negatives.join(" NOT ")))
    }
}

/// Converts a single bare word into a safe FTS5 fragment, honoring the `term*`
/// prefix operator (needs >=2 leading alphanumeric chars, else literal). The
/// word is split on non-alphanumerics like the plain-text path so symbols stay
/// safe; an all-symbol word yields no fragment.
fn fts_fragment_for_word(word: &str) -> Option<String> {
    let wants_prefix = word.ends_with('*');
    let core = if wants_prefix {
        word.trim_end_matches('*')
    } else {
        word
    };

    // Keep only alphanumeric runs, mirroring the plain-text tokenizer so the
    // quoted FTS term never contains FTS5-significant punctuation.
    let cleaned = core
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if cleaned.is_empty() {
        return None;
    }

    if wants_prefix && cleaned.len() == 1 && cleaned[0].chars().count() >= 2 {
        // term* → prefix query: "term"*
        return Some(format!("{}*", fts_quote_phrase_term(cleaned[0])));
    }

    // Otherwise treat the cleaned parts as a phrase (handles symbol-joined
    // words and prefix tokens that did not qualify, which become literal).
    Some(
        cleaned
            .into_iter()
            .map(fts_quote_phrase_term)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

// --- Merge parsed field operators into caller refinements ---

/// Merges parsed field operators into any caller-supplied refinements per each
/// field's multiplicity rule: apps/sources accumulate (set, dedup) and the date
/// slot is overwritten by parsed date operators (last-write-wins).
pub(super) fn merge_parsed_field_operators(
    base: Option<SearchCaptureRefinements>,
    parsed: &ParsedQuery,
) -> SearchCaptureRefinements {
    let mut refinements = base.unwrap_or_default();

    for app in &parsed.apps {
        if !refinements.apps.contains(app) {
            refinements.apps.push(app.clone());
        }
    }
    for source in &parsed.audio_sources {
        if !refinements.audio_sources.contains(source) {
            refinements.audio_sources.push(source.clone());
        }
    }
    if parsed.screen_source {
        refinements.screen_source = true;
    }
    if let Some(date_range) = &parsed.date_range {
        refinements.date_range = Some(date_range.clone());
    }

    refinements
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchDateRangeOrigin;

    #[test]
    fn search_refinement_dates_normalize_to_utc() {
        let normalized = normalize_search_refinements(Some(SearchCaptureRefinements {
            date_range: Some(SearchDateRangeRefinement {
                start_at: "2026-05-17T04:59:00-05:00".to_string(),
                end_at: "2026-05-17T05:01:00-05:00".to_string(),
                origin: Some(SearchDateRangeOrigin::LastHour),
            }),
            apps: Vec::new(),
            window_title: None,
            audio_sources: Vec::new(),
            screen_source: false,
        }))
        .expect("refinements should not error")
        .expect("refinements should normalize");

        let range = normalized
            .date_range
            .as_ref()
            .expect("date range should be present");
        assert_eq!(range.start_at, "2026-05-17T09:59:00Z");
        assert_eq!(range.end_at, "2026-05-17T10:01:00Z");
        assert_eq!(
            normalized
                .applied
                .date_range
                .as_ref()
                .map(|range| (range.start_at.as_str(), range.end_at.as_str())),
            Some(("2026-05-17T09:59:00Z", "2026-05-17T10:01:00Z"))
        );
    }

    #[test]
    fn plain_text_query_has_no_operators_and_matches_plain_fts() {
        // A query with no operators must behave exactly as the plain-text path:
        // residual equals the input, no refinements, and the FTS body is exactly
        // what the legacy plain-text translator produces.
        let parsed = parse_search_query("hello world");
        assert!(parsed.errors.is_empty());
        assert!(parsed.apps.is_empty());
        assert!(parsed.audio_sources.is_empty());
        assert!(parsed.date_range.is_none());
        assert_eq!(parsed.residual_query, "hello world");
        assert_eq!(
            parsed.fts_body,
            fts_query_for_plain_text(&normalize_query("hello world"))
        );
        assert_eq!(parsed.fts_body, "\"hello\" \"world\"");
    }

    #[test]
    fn quoted_phrase_body_operator_forces_literal_phrase() {
        let parsed = parse_search_query("\"hello world\"");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.fts_body, "(\"hello world\")");
    }

    #[test]
    fn quoted_phrase_preserves_doubled_quotes_as_literal_quote() {
        // `""` inside a quoted run is an escaped literal `"`, not a close+reopen.
        // `"he said ""hi"""` must parse the phrase `he said "hi"` rather than
        // collapsing the doubled quotes away into `he said hi`.
        let parsed = parse_search_query("\"he said \"\"hi\"\"\"");
        assert!(
            parsed.errors.is_empty(),
            "unexpected parse errors: {:?}",
            parsed.errors
        );
        assert_eq!(parsed.fts_body, "(\"he said \"\"hi\"\"\")");
    }

    #[test]
    fn exclusion_body_operator_with_positive_term_is_fts_not() {
        let parsed = parse_search_query("error -warning");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.fts_body, "(\"error\" NOT \"warning\")");
    }

    #[test]
    fn uppercase_or_body_operator_splits_groups_lowercase_or_is_literal() {
        let parsed = parse_search_query("foo OR bar");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.fts_body, "(\"foo\") OR (\"bar\")");

        // lowercase `or` is a literal AND term, never a group split.
        let lower = parse_search_query("foo or bar");
        assert!(lower.errors.is_empty());
        assert_eq!(lower.fts_body, "\"foo\" \"or\" \"bar\"");
    }

    #[test]
    fn dangling_or_body_operator_is_a_parse_error() {
        // Leading, trailing, and doubled `OR` all leave an empty AND-group.
        // Strict validation (ADR 0019) rejects them instead of silently
        // rewriting into a broader valid search; no FTS body is produced.
        for query in ["foo OR", "OR foo", "foo OR OR bar"] {
            let parsed = parse_search_query(query);
            assert_eq!(
                parsed.errors.len(),
                1,
                "expected one dangling_or error for {query:?}, got {:?}",
                parsed.errors
            );
            assert_eq!(parsed.errors[0].kind, "dangling_or");
            assert!(
                parsed.fts_body.is_empty(),
                "dangling OR should produce no FTS body for {query:?}, got {:?}",
                parsed.fts_body
            );
        }

        // A well-formed OR with terms on both sides still parses cleanly.
        let ok = parse_search_query("foo OR bar");
        assert!(ok.errors.is_empty());
        assert_eq!(ok.fts_body, "(\"foo\") OR (\"bar\")");
    }

    #[test]
    fn prefix_body_operator_requires_two_leading_chars() {
        let parsed = parse_search_query("term*");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.fts_body, "(\"term\"*)");

        // A single leading char does not qualify; it stays a literal term.
        let short = parse_search_query("a*");
        assert!(short.errors.is_empty());
        assert_eq!(short.fts_body, "(\"a\")");
    }

    #[test]
    fn app_field_operator_desugars_into_any_app_refinement() {
        let parsed = parse_search_query("app:Safari report");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.apps.len(), 1);
        assert!(matches!(parsed.apps[0].kind, SearchAppRefinementKind::Any));
        assert_eq!(parsed.apps[0].value, "Safari");
        assert_eq!(parsed.residual_query, "report");
        assert_eq!(parsed.fts_body, "\"report\"");
    }

    #[test]
    fn app_field_operator_supports_quoted_multiword_and_reverse_dns() {
        let quoted = parse_search_query("app:\"Google Chrome\"");
        assert!(quoted.errors.is_empty());
        assert_eq!(quoted.apps.len(), 1);
        assert_eq!(quoted.apps[0].value, "Google Chrome");
        assert!(matches!(quoted.apps[0].kind, SearchAppRefinementKind::Any));

        // A reverse-DNS-looking value still works via the Any match kind because
        // a recognized `app:` value is never re-split as a field operator.
        let bundle = parse_search_query("app:com.google.Chrome");
        assert!(bundle.errors.is_empty());
        assert_eq!(bundle.apps.len(), 1);
        assert_eq!(bundle.apps[0].value, "com.google.Chrome");
        assert!(matches!(bundle.apps[0].kind, SearchAppRefinementKind::Any));
    }

    #[test]
    fn multiple_app_operators_accumulate_and_dedupe() {
        let parsed = parse_search_query("app:Safari app:Chrome app:Safari");
        assert!(parsed.errors.is_empty());
        let values = parsed
            .apps
            .iter()
            .map(|app| app.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["Safari", "Chrome"]);
    }

    #[test]
    fn source_field_operator_maps_to_audio_source_kinds() {
        let mic = parse_search_query("source:mic");
        assert!(mic.errors.is_empty());
        assert_eq!(mic.audio_sources, vec![AudioSegmentSourceKind::Microphone]);

        let both = parse_search_query("source:mic source:system");
        assert!(both.errors.is_empty());
        assert_eq!(
            both.audio_sources,
            vec![
                AudioSegmentSourceKind::Microphone,
                AudioSegmentSourceKind::SystemAudio
            ]
        );
    }

    #[test]
    fn unknown_source_value_is_in_band_error() {
        let parsed = parse_search_query("source:bluetooth");
        assert!(parsed.audio_sources.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.errors[0].kind, "unknown_source");
    }

    #[test]
    fn source_screen_operator_sets_screen_source_without_audio_sources() {
        let parsed = parse_search_query("source:screen meeting");
        assert!(parsed.errors.is_empty());
        assert!(parsed.screen_source);
        assert!(parsed.audio_sources.is_empty());
    }

    #[test]
    fn screen_and_audio_source_conflict_is_in_band_error() {
        let parsed = parse_search_query("source:screen source:mic");
        assert!(parsed.errors.is_empty());

        let errors =
            normalize_search_refinements(Some(merge_parsed_field_operators(None, &parsed)))
                .expect("conflict should not throw")
                .expect_err("screen + audio source should surface in-band parse errors");
        assert!(
            errors
                .iter()
                .any(|error| error.kind == "screen_audio_source_conflict"),
            "expected a screen_audio_source_conflict parse error, got {errors:?}"
        );
    }

    #[test]
    fn date_field_operators_resolve_to_a_single_overwriting_slot() {
        let parsed = parse_search_query("after:2026-01-01 before:2026-01-31");
        assert!(parsed.errors.is_empty());
        let range = parsed.date_range.expect("date range should be set");
        assert!(range.start_at.starts_with("2026-01-01T00:00:00"));
        assert!(range.end_at.starts_with("2026-01-31T23:59:59"));

        // `date:` writes both bounds; last write wins per slot.
        let day = parse_search_query("after:2020-01-01 date:2026-05-17");
        assert!(day.errors.is_empty());
        let day_range = day.date_range.expect("date range should be set");
        assert!(day_range.start_at.starts_with("2026-05-17T00:00:00"));
        assert!(day_range.end_at.starts_with("2026-05-17T23:59:59"));
    }

    #[test]
    fn bad_date_value_is_in_band_error() {
        let parsed = parse_search_query("after:notadate");
        assert!(parsed.date_range.is_none());
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.errors[0].kind, "bad_date");
        assert_eq!(parsed.errors[0].token, "after:notadate");
    }

    #[test]
    fn unknown_key_value_tokens_stay_literal_body_text() {
        // URL/code/error searches must keep working: only the known keys are
        // field operators, every other `key:value` is literal body text.
        for raw in ["http://github.com", "error:404", "fix: bug"] {
            let parsed = parse_search_query(raw);
            assert!(
                parsed.apps.is_empty()
                    && parsed.audio_sources.is_empty()
                    && parsed.date_range.is_none(),
                "`{raw}` must not desugar into any field operator"
            );
            assert!(parsed.errors.is_empty(), "`{raw}` must not error");
            assert!(
                parsed
                    .residual_query
                    .contains(raw.split(' ').next().unwrap()),
                "`{raw}` should remain in the residual body, got {:?}",
                parsed.residual_query
            );
        }

        // The literal http URL still produces a non-empty (safe) FTS body.
        let url = parse_search_query("http://github.com");
        assert!(!url.fts_body.is_empty());
    }

    #[test]
    fn unbalanced_quote_is_in_band_error() {
        let parsed = parse_search_query("\"unterminated phrase");
        assert!(
            parsed
                .errors
                .iter()
                .any(|error| error.kind == "unbalanced_quote"),
            "expected an unbalanced_quote error, got {:?}",
            parsed.errors
        );
    }

    #[test]
    fn pure_negation_without_positive_term_is_in_band_error() {
        let parsed = parse_search_query("-foo");
        assert!(
            parsed
                .errors
                .iter()
                .any(|error| error.kind == "pure_negation"),
            "expected a pure_negation error, got {:?}",
            parsed.errors
        );
    }

    #[test]
    fn error_spans_are_character_offsets_into_the_raw_query() {
        // Use a multi-byte prefix to prove spans are character (not byte) offsets.
        let parsed = parse_search_query("café source:bluetooth");
        let error = parsed
            .errors
            .iter()
            .find(|error| error.kind == "unknown_source")
            .expect("unknown_source error");
        // "café " is 5 characters; the 16-char token spans chars [5, 21).
        assert_eq!(error.start, 5);
        assert_eq!(error.end, 21);
        assert_eq!(error.token, "source:bluetooth");
    }
}
