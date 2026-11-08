//! Wire strings headed for the approval window: scope checks and the text
//! cleanup every client-supplied string runs through.

use app_infra::brokered_access::BrokerGrantScope;

use super::{AuthorizationChannelRequest, CLIENT_LABEL_MAX_CHARS};

/// Does the selected scope cover the request's minimum? Both sides are wire
/// spellings, so both route through [`BrokerGrantScope`] rather than being
/// compared as strings — an unrecognized spelling is a rejection, never a
/// coincidental match.
pub(super) fn scope_satisfies_minimum(selected: &str, minimum: &str) -> bool {
    match (
        BrokerGrantScope::from_wire_name(selected),
        BrokerGrantScope::from_wire_name(minimum),
    ) {
        (Some(selected), Some(minimum)) => selected.covers(&minimum),
        _ => false,
    }
}

/// The client name is wire input from whoever connected to the socket, and it
/// lands in the approval window's requester chip. Control characters and
/// unbounded length would let that name restructure the sheet — fabricating the
/// app's own copy, or pushing the consent controls out of view. Same
/// normalization app-infra applies to a stored grant label
/// (`display_client_label`), plus a cap.
/// Characters that render as nothing, or that reorder the text around them.
///
/// `char::is_control()` covers only the C0/C1 blocks (Unicode `Cc`), so bidi
/// overrides/isolates (U+202E, U+2066..=U+2069) and zero-width characters (U+200B,
/// U+FEFF, tag characters) sail straight past it — they are `Cf`, and none of them
/// is `White_Space`. In a consent prompt that is not cosmetic: a label made
/// entirely of zero-width characters is non-empty by `is_empty()`, so it defeats
/// the "An unnamed local tool" fallback below and the window names no requester at
/// all; and an override reverses the display order of the copy around it.
///
/// The set is Unicode's `Default_Ignorable_Code_Point` — the closed, stable list
/// of "this is meant to render as nothing", which is exactly the property that
/// matters here and covers the bidi controls, the zero-widths, the fillers
/// (U+3164 HANGUL FILLER is the classic invisible-name trick) and the variation
/// selectors in one go — plus the two blank-glyph outliers it leaves out:
/// U+2800 BRAILLE PATTERN BLANK and the U+FFF9..=U+FFFB annotation controls.
/// Enumerating "invisible" any other way is a blocklist that leaks; this one is
/// a Unicode property with a fixed definition.
pub(super) fn is_invisible_or_reordering(ch: char) -> bool {
    matches!(ch,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{2800}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}'
        | '\u{115F}'..='\u{1160}'
        | '\u{17B4}'..='\u{17B5}'
        | '\u{180B}'..='\u{180F}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FE00}'..='\u{FE0F}'
        | '\u{FFF0}'..='\u{FFFB}'
        | '\u{1BCA0}'..='\u{1BCA3}'
        | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// The one collapse every wire string headed for the approval window runs
/// through: invisibles and reordering controls dropped, controls folded to the
/// space they render as, whitespace runs collapsed, and the result capped so it
/// cannot push the consent controls off the sheet.
pub(super) fn collapsed_wire_text(text: &str) -> String {
    let cleaned = text
        .chars()
        .filter(|ch| !is_invisible_or_reordering(*ch))
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.chars().count() <= CLIENT_LABEL_MAX_CHARS {
        return cleaned;
    }
    cleaned
        .chars()
        .take(CLIENT_LABEL_MAX_CHARS)
        .chain(std::iter::once('…'))
        .collect()
}

pub(super) fn client_label_display(label: &str) -> String {
    let cleaned = collapsed_wire_text(label);
    if cleaned.is_empty() {
        return "An unnamed local tool".to_string();
    }
    cleaned
}

/// The command shown in the window's "Requested via `mnema …`" line is wire input
/// exactly like the label — same collapse, no name fallback (an empty command
/// just renders as `mnema`).
pub(super) fn command_display(command: &str) -> String {
    collapsed_wire_text(command)
}

/// Could this request ever become a grant? A client name that normalizes to
/// nothing cannot key a permission row (`BrokerClientIdentity::new` rejects it,
/// which used to happen only AFTER the user consented), and an unrecognized scope
/// spelling can never satisfy `scope_satisfies_minimum`. Checked before any
/// window opens.
pub(super) fn request_is_well_formed(request: &AuthorizationChannelRequest) -> bool {
    app_infra::brokered_access::normalize_client_label(&request.client.label).is_some()
        && BrokerGrantScope::from_wire_name(&request.scope.minimum).is_some()
        && BrokerGrantScope::from_wire_name(&request.scope.preferred).is_some()
}
