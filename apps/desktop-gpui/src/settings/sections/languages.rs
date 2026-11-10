//! The language pickers' option lists, ported verbatim from
//! `transcription-languages.ts` and `ocr-languages.ts`: `(stored code, label)`,
//! labelled "Name (code)" so a search matches either. Every transcription
//! provider gets the same list (Svelte has no per-provider set); a provider
//! switch leaves the stored language alone.

/// `TRANSCRIPTION_LANGUAGE_OPTIONS`: `auto` (automatic detection) first.
pub const TRANSCRIPTION: [(&str, &str); 65] = [
    ("auto", "Auto-detect"),
    ("en", "English (en)"),
    ("zh", "Chinese (zh)"),
    ("de", "German (de)"),
    ("es", "Spanish (es)"),
    ("ru", "Russian (ru)"),
    ("ko", "Korean (ko)"),
    ("fr", "French (fr)"),
    ("ja", "Japanese (ja)"),
    ("pt", "Portuguese (pt)"),
    ("tr", "Turkish (tr)"),
    ("pl", "Polish (pl)"),
    ("ca", "Catalan (ca)"),
    ("nl", "Dutch (nl)"),
    ("ar", "Arabic (ar)"),
    ("sv", "Swedish (sv)"),
    ("it", "Italian (it)"),
    ("id", "Indonesian (id)"),
    ("hi", "Hindi (hi)"),
    ("fi", "Finnish (fi)"),
    ("vi", "Vietnamese (vi)"),
    ("he", "Hebrew (he)"),
    ("uk", "Ukrainian (uk)"),
    ("el", "Greek (el)"),
    ("ms", "Malay (ms)"),
    ("cs", "Czech (cs)"),
    ("ro", "Romanian (ro)"),
    ("da", "Danish (da)"),
    ("hu", "Hungarian (hu)"),
    ("ta", "Tamil (ta)"),
    ("no", "Norwegian (no)"),
    ("th", "Thai (th)"),
    ("ur", "Urdu (ur)"),
    ("hr", "Croatian (hr)"),
    ("bg", "Bulgarian (bg)"),
    ("lt", "Lithuanian (lt)"),
    ("la", "Latin (la)"),
    ("mi", "Maori (mi)"),
    ("ml", "Malayalam (ml)"),
    ("cy", "Welsh (cy)"),
    ("sk", "Slovak (sk)"),
    ("te", "Telugu (te)"),
    ("fa", "Persian (fa)"),
    ("lv", "Latvian (lv)"),
    ("bn", "Bengali (bn)"),
    ("sr", "Serbian (sr)"),
    ("az", "Azerbaijani (az)"),
    ("sl", "Slovenian (sl)"),
    ("kn", "Kannada (kn)"),
    ("et", "Estonian (et)"),
    ("mk", "Macedonian (mk)"),
    ("eu", "Basque (eu)"),
    ("is", "Icelandic (is)"),
    ("hy", "Armenian (hy)"),
    ("ne", "Nepali (ne)"),
    ("sw", "Swahili (sw)"),
    ("gl", "Galician (gl)"),
    ("mr", "Marathi (mr)"),
    ("pa", "Punjabi (pa)"),
    ("si", "Sinhala (si)"),
    ("km", "Khmer (km)"),
    ("sq", "Albanian (sq)"),
    ("af", "Afrikaans (af)"),
    ("ka", "Georgian (ka)"),
    ("be", "Belarusian (be)"),
];

/// `OCR_LANGUAGE_OPTIONS`: Tesseract `traineddata` codes, one per pick.
pub const OCR: [(&str, &str); 9] = [
    ("eng", "English (eng)"),
    ("fra", "French (fra)"),
    ("deu", "German (deu)"),
    ("spa", "Spanish (spa)"),
    ("ita", "Italian (ita)"),
    ("por", "Portuguese (por)"),
    ("nld", "Dutch (nld)"),
    ("jpn", "Japanese (jpn)"),
    ("chi_sim", "Chinese, Simplified (chi_sim)"),
];

/// `(selected index, placeholder)` for a stored code: a code outside the list
/// (hand-edited settings, a `+`-joined Tesseract pair) selects nothing and shows as is.
pub fn pick(options: &[(&str, &str)], stored: &str) -> (Option<usize>, String) {
    match options.iter().position(|(code, _)| *code == stored) {
        Some(ix) => (Some(ix), String::new()),
        None => (None, stored.to_string()),
    }
}
