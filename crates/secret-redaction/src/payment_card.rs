//! Card-number (PAN) validation for the `PaymentCard` detector. The regex finds
//! digit runs; this module narrows a run to the longest sub-run of digit groups
//! that is a plausible card number, so adjacent expiry/CVV/order digits joined
//! onto the same OCR visual line neither mask nor widen the redaction.

/// Byte offsets, relative to `matched`, of the longest contiguous run of digit
/// groups that validates as a card number, or `None` when no sub-run does.
pub(crate) fn refine_pan_match(matched: &str) -> Option<(usize, usize)> {
    let groups = digit_groups(matched);
    // ponytail: O(groups²) scan; a 19-digit match has at most ~10 groups.
    for len in (1..=groups.len()).rev() {
        for window in groups.windows(len) {
            let texts: Vec<&str> = window.iter().map(|&(s, e)| &matched[s..e]).collect();
            if is_valid_pan_digits(&texts.concat(), &texts) {
                return Some((window[0].0, window[len - 1].1));
            }
        }
    }
    None
}

fn digit_groups(matched: &str) -> Vec<(usize, usize)> {
    let mut groups = Vec::new();
    let mut start = None;
    for (index, c) in matched.char_indices() {
        match (c.is_ascii_digit(), start) {
            (true, None) => start = Some(index),
            (false, Some(s)) => {
                groups.push((s, index));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        groups.push((s, matched.len()));
    }
    groups
}

fn is_valid_pan_digits(digits: &str, groups: &[&str]) -> bool {
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && separator_groups_ok(groups)
        && issuer_accepts(digits)
        && luhn_valid(digits)
        // ponytail: sequential strings like 1234567890123456 are already
        // rejected by the issuer table; only the all-same-digit guard remains.
        && !digits.bytes().all(|b| b == digits.as_bytes()[0])
}

/// Card groupings are 4-4-4-4, 4-6-5, 4-4-4-4-3: every group but the last has
/// at least four digits. Rejects date runs like `2026-05-17`.
fn separator_groups_ok(groups: &[&str]) -> bool {
    groups
        .split_last()
        .is_some_and(|(_, leading)| leading.iter().all(|g| g.len() >= 4))
}

fn issuer_accepts(digits: &str) -> bool {
    let prefix = |n: usize| digits.get(..n).and_then(|p| p.parse::<u32>().ok());
    let in_range = |n: usize, lo: u32, hi: u32| prefix(n).is_some_and(|p| (lo..=hi).contains(&p));
    let len = digits.len();
    let visa = digits.starts_with('4') && matches!(len, 13 | 16 | 19);
    let mastercard = (in_range(2, 51, 55) || in_range(4, 2221, 2720)) && len == 16;
    let amex = (in_range(2, 34, 34) || in_range(2, 37, 37)) && len == 15;
    let discover = (in_range(4, 6011, 6011) || in_range(2, 65, 65) || in_range(3, 644, 649))
        && matches!(len, 16 | 19);
    let jcb = in_range(4, 3528, 3589) && (16..=19).contains(&len);
    let diners = (in_range(2, 36, 36)
        || in_range(3, 300, 305)
        || in_range(4, 3095, 3095)
        || in_range(2, 38, 39))
        && (14..=19).contains(&len);
    let unionpay = in_range(2, 62, 62) && (16..=19).contains(&len);
    visa || mastercard || amex || discover || jcb || diners || unionpay
}

fn luhn_valid(digits: &str) -> bool {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let sum: u32 = digits
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0');
            if i % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            } else {
                d
            }
        })
        .sum();
    sum % 10 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups_of(text: &str) -> Vec<&str> {
        text.split([' ', '-']).collect()
    }

    fn refined(text: &str) -> Option<&str> {
        refine_pan_match(text).map(|(s, e)| &text[s..e])
    }

    #[test]
    fn luhn() {
        assert!(luhn_valid("4111111111111111"));
        assert!(!luhn_valid("4111111111111112"));
        assert!(luhn_valid("378282246310005"));
        assert!(!luhn_valid(""));
    }

    #[test]
    fn issuer_table() {
        assert!(issuer_accepts("4111111111111111"));
        assert!(!issuer_accepts("411111111111111"));
        assert!(issuer_accepts("378282246310005"));
        assert!(!issuer_accepts("3782822463100050"));
        assert!(!issuer_accepts("1700000000000"));
        assert!(issuer_accepts("6011111111111117"));
        assert!(issuer_accepts("2221000000000009"));
        assert!(!issuer_accepts("2721000000000000"));
    }

    #[test]
    fn separator_groups() {
        assert!(separator_groups_ok(&groups_of("4111 1111 1111 1111")));
        assert!(separator_groups_ok(&groups_of("4111-1111-1111-1111")));
        assert!(separator_groups_ok(&groups_of("3782 822463 10005")));
        assert!(!separator_groups_ok(&groups_of("41 11 11 11 11 11 11 11")));
        assert!(separator_groups_ok(&groups_of("4111111111111111")));
    }

    #[test]
    fn refine() {
        assert_eq!(refined("4111 1111 1111 1111"), Some("4111 1111 1111 1111"));
        assert_eq!(
            refined("4111 1111 1111 1111 09"),
            Some("4111 1111 1111 1111")
        );
        assert_eq!(
            refined("4111 1111 1111 1111 123"),
            Some("4111 1111 1111 1111")
        );
        assert_eq!(
            refined("42 4111 1111 1111 1111"),
            Some("4111 1111 1111 1111")
        );
        assert_eq!(refined("2026-05-17 2026-05-18"), None);
        assert_eq!(refined("1716478783123"), None);
        assert_eq!(refined("4111111111111112"), None);
    }
}
