//! Secret redaction for every string that can reach a log, an error, a
//! report, a checkpoint or a UI payload.
//!
//! # Why this is its own module
//!
//! A BYOK key is only safe if *every* path that can print a string is forced
//! through one function. Scattering `format!("{err}")` around the Gemini
//! client is how a provider error body containing the echoed key ends up in a
//! crash report. The rule this module encodes is therefore blunt: provider
//! transports never return raw bodies to callers, they return
//! [`redact`]-ed text, and the tests scan for a canary in every artifact.
//!
//! Redaction is deliberately *lossy and irreversible*. It never emits a hash
//! of the secret either: a hash of a 39-character Gemini key is a perfectly
//! good oracle for anyone who can guess candidates, so a masked reference is
//! a slot name and a length class, never a digest.

/// What a masked credential reference looks like everywhere it is displayed.
///
/// Deliberately not the first/last characters of the key: a prefix leaks the
/// project, and a suffix plus a length is enough to confirm a guess.
pub const MASKED_CREDENTIAL: &str = "****";

/// Upper bound on any single redacted string this crate produces. Provider
/// error bodies are attacker-influenced input; they are truncated before they
/// are ever stored.
pub const MAX_REDACTED_BYTES: usize = 512;

/// Header and query names whose *values* are always removed, whatever they
/// look like.
const SENSITIVE_KEYS: [&str; 9] = [
    "authorization",
    "x-goog-api-key",
    "x-api-key",
    "api_key",
    "apikey",
    "key",
    "access_token",
    "cookie",
    "set-cookie",
];

/// Literal prefixes that identify a credential no matter where they appear.
const SECRET_PREFIXES: [&str; 4] = ["AIza", "sk-", "Bearer ", "ya29."];

/// Redacts anything that looks like a credential and bounds the result.
///
/// This is intentionally over-eager. Losing a diagnostic detail is cheap;
/// printing a key once is not recoverable.
pub fn redact(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(MAX_REDACTED_BYTES));
    for (index, raw_line) in input.lines().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&redact_line(raw_line));
        if out.len() > MAX_REDACTED_BYTES {
            break;
        }
    }
    truncate_on_char_boundary(out)
}

/// Redacts one line.
///
/// Two independent passes, because the two failure modes are different. A
/// *named* secret (`Authorization:`, `apiKey=`) is sensitive because of where
/// it sits, whatever it looks like — so once a sensitive name is followed by
/// a separator, everything up to the next structural delimiter is dropped. A
/// *shaped* secret (`AIza…`, `sk-…`, a long opaque run) is sensitive wherever
/// it appears, including inside a JSON blob with no spaces in it.
///
/// The pass is deliberately over-eager: it will sometimes swallow an adjacent
/// harmless field. Losing `status=403` from a diagnostic is a bad afternoon;
/// printing a key once is not recoverable.
fn redact_line(line: &str) -> String {
    let characters: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut index = 0;
    let mut previous_name: Option<String> = None;
    let mut after_separator = false;
    let mut suppressing = false;

    while index < characters.len() {
        let character = characters[index];
        if is_run_character(character) {
            let start = index;
            while index < characters.len() && is_run_character(characters[index]) {
                index += 1;
            }
            let run: String = characters[start..index].iter().collect();
            if suppressing {
                continue;
            }
            if after_separator && previous_name.as_deref().is_some_and(is_sensitive_name) {
                out.push_str(MASKED_CREDENTIAL);
                suppressing = true;
                continue;
            }
            if is_secret_shaped(&run) {
                out.push_str(MASKED_CREDENTIAL);
            } else {
                out.push_str(&run);
            }
            previous_name = Some(run);
            after_separator = false;
            continue;
        }

        index += 1;
        if matches!(character, ',' | ';' | '}') {
            // A structural delimiter ends a suppressed value, so the rest of
            // a JSON object stays readable.
            suppressing = false;
            previous_name = None;
            after_separator = false;
            out.push(character);
            continue;
        }
        if suppressing {
            continue;
        }
        match character {
            ':' | '=' => after_separator = true,
            '"' | '\'' | ' ' | '\t' => {}
            _ => {
                after_separator = false;
                previous_name = None;
            }
        }
        out.push(character);
    }
    out
}

fn is_run_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
}

/// Names whose values are secret regardless of shape.
fn is_sensitive_name(name: &str) -> bool {
    let folded: String = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect();
    SENSITIVE_KEYS
        .iter()
        .any(|candidate| folded == candidate.replace(['-', '_'], ""))
        || [
            "key",
            "token",
            "secret",
            "password",
            "auth",
            "credential",
            "cookie",
        ]
        .iter()
        .any(|needle| folded.contains(needle))
}

fn is_secret_shaped(run: &str) -> bool {
    SECRET_PREFIXES
        .iter()
        .any(|prefix| run.starts_with(prefix.trim_end()))
        || looks_like_opaque_credential(run)
}

/// A long, high-entropy, punctuation-free run is treated as a credential.
///
/// The 32-character floor also matches a sha256 hex digest. That is
/// acceptable: this function only ever runs over provider diagnostics, never
/// over evidence records, where digests are structured fields that never pass
/// through here.
fn looks_like_opaque_credential(run: &str) -> bool {
    run.len() >= 32
        && run
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        && run.bytes().any(|byte| byte.is_ascii_digit())
        && run.bytes().any(|byte| byte.is_ascii_alphabetic())
}

fn truncate_on_char_boundary(mut text: String) -> String {
    if text.len() <= MAX_REDACTED_BYTES {
        return text;
    }
    let mut end = MAX_REDACTED_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push('…');
    text
}

/// The only description of a stored credential this project ever produces.
///
/// It names the slot and nothing else: no prefix, no suffix, no length, no
/// digest. `present` is the single bit a UI legitimately needs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MaskedCredential {
    pub slot: String,
    pub present: bool,
    /// Always [`MASKED_CREDENTIAL`]. Present as a field so a UI that renders
    /// the struct cannot accidentally render a value instead.
    pub masked: String,
}

impl MaskedCredential {
    pub fn new(slot: impl Into<String>, present: bool) -> Self {
        Self {
            slot: slot.into(),
            present,
            masked: MASKED_CREDENTIAL.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_google_api_key_never_survives_redaction() {
        let canary = "AIzaSyD-canary-0000000000000000000000";
        let text = format!("request failed: key={canary} status=403");
        let redacted = redact(&text);
        assert!(!redacted.contains(canary), "{redacted}");
        assert!(redacted.contains(MASKED_CREDENTIAL));
    }

    #[test]
    fn an_authorization_header_line_is_removed_by_name_not_by_shape() {
        // The value here is short and unremarkable; only the header name
        // makes it sensitive, which is exactly the case a shape-based filter
        // misses.
        let redacted = redact("Authorization: Bearer abc");
        assert!(!redacted.contains("abc"), "{redacted}");
    }

    #[test]
    fn a_provider_error_body_is_bounded_and_stripped() {
        let body = format!(
            "{{\"error\":{{\"message\":\"API key not valid\",\"apiKey\":\"{}\"}}}}\n{}",
            "sk-0123456789abcdef0123456789abcdef",
            "x".repeat(4096)
        );
        let redacted = redact(&body);
        assert!(redacted.len() <= MAX_REDACTED_BYTES + 4);
        assert!(!redacted.contains("sk-0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn ordinary_diagnostics_are_left_readable() {
        let redacted = redact("page 12 timed out after 30s");
        assert_eq!(redacted, "page 12 timed out after 30s");
    }

    #[test]
    fn a_masked_credential_reveals_only_the_slot_and_presence() {
        let masked = MaskedCredential::new("gemini-default", true);
        let json = serde_json::to_string(&masked).unwrap();
        assert_eq!(
            json,
            r#"{"slot":"gemini-default","present":true,"masked":"****"}"#
        );
    }
}
