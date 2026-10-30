//! Commands for Apple's `security` tool and the meaning of its replies; no I/O.

use std::ffi::OsString;

use zeroize::Zeroizing;

use crate::error::local_data::KeychainError;

pub const PROGRAM: &str = "/usr/bin/security";

pub const MAX_LINE: usize = 4094;

const NOT_FOUND: i32 = -25_300;
const NO_INTERACTION: i32 = -25_308;
const AUTH_FAILED: i32 = -25_293;
const CANCELLED: i32 = -128;

/// A Keychain item: service, account, and a keychain file or the default ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeychainItem<'a> {
    pub service: &'a str,
    pub account: &'a str,
    pub keychain: Option<&'a str>,
}

/// What a finished `security` run means.
#[derive(Debug)]
pub enum SecurityExit {
    Done,
    Missing,
    Error(KeychainError),
}

impl SecurityExit {
    /// Success only: a missing item is a failure here (a write, say).
    ///
    /// # Errors
    /// The failure, or [`KeychainError::Exited`] with `errSecItemNotFound`.
    pub fn required(self) -> Result<(), KeychainError> {
        match self {
            SecurityExit::Done => Ok(()),
            SecurityExit::Missing => Err(KeychainError::Exited { status: NOT_FOUND }),
            SecurityExit::Error(err) => Err(err),
        }
    }
}

/// The arguments of `find-generic-password` that print the value to stderr.
#[must_use]
pub fn find_arguments(item: &KeychainItem<'_>) -> Vec<OsString> {
    with_keychain(
        [
            "find-generic-password",
            "-s",
            item.service,
            "-a",
            item.account,
            "-g",
        ],
        item,
    )
}

/// The arguments of `delete-generic-password`.
#[must_use]
pub fn delete_arguments(item: &KeychainItem<'_>) -> Vec<OsString> {
    with_keychain(
        [
            "delete-generic-password",
            "-s",
            item.service,
            "-a",
            item.account,
        ],
        item,
    )
}

fn with_keychain<const N: usize>(
    words: [&str; N],
    item: &KeychainItem<'_>,
) -> Vec<OsString> {
    words
        .into_iter()
        .chain(item.keychain)
        .map(OsString::from)
        .collect()
}

fn line_parts(item: &KeychainItem<'_>) -> Result<(String, String), KeychainError> {
    let head = quote(item.service)
        .zip(quote(item.account))
        .map(|(service, account)| {
            format!("add-generic-password -U -s {service} -a {account} -X ")
        })
        .ok_or(KeychainError::BadName)?;
    let tail = item
        .keychain
        .map_or(Some(String::new()), |keychain| {
            quote(keychain).map(|keychain| format!(" {keychain}"))
        })
        .ok_or(KeychainError::BadName)?;
    Ok((head, tail))
}

/// The longest value in bytes [`add_line`] takes for this item.
///
/// # Errors
/// [`KeychainError::BadName`] for a name one line cannot carry.
pub fn room(item: &KeychainItem<'_>) -> Result<usize, KeychainError> {
    line_parts(item).map(|(head, tail)| room_beside(&head, &tail))
}

fn room_beside(head: &str, tail: &str) -> usize {
    MAX_LINE.saturating_sub(head.len() + tail.len()) / 2
}

/// The stdin line for `security -i` that adds or updates the item with
/// `secret`.
///
/// # Errors
/// [`KeychainError::BadName`] for a name one line cannot carry,
/// [`KeychainError::TooLong`] when the line would be cut by `security`.
pub fn add_line(
    item: &KeychainItem<'_>,
    secret: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KeychainError> {
    let (head, tail) = line_parts(item)?;
    match (secret.len(), room_beside(&head, &tail)) {
        (len, max) if len > max => Err(KeychainError::TooLong { len, max }),
        (len, _) => Ok(appended(
            Zeroizing::new(Vec::with_capacity(head.len() + 2 * len + tail.len() + 1)),
            |line| {
                line.extend_from_slice(head.as_bytes());
                hex_into(line, secret);
                line.extend_from_slice(tail.as_bytes());
                line.push(b'\n');
            },
        )),
    }
}

// The buffer holds a secret, so it is filled in place at its final capacity.
fn appended(
    mut line: Zeroizing<Vec<u8>>,
    fill: impl FnOnce(&mut Vec<u8>),
) -> Zeroizing<Vec<u8>> {
    fill(&mut line);
    line
}

/// One argument for `security -i`: in double quotes, `\` and `"` escaped.
#[must_use]
pub fn quote(arg: &str) -> Option<String> {
    let escaped = arg.chars().flat_map(|c| {
        ['\\', c]
            .into_iter()
            .skip(usize::from(!matches!(c, '\\' | '"')))
    });
    (!arg.contains(['\n', '\0'])).then(|| {
        std::iter::once('"')
            .chain(escaped)
            .chain(std::iter::once('"'))
            .collect()
    })
}

/// Lowercase hex digits of `bytes`, appended without growing `out` past need.
pub fn hex_into(out: &mut Vec<u8>, bytes: &[u8]) {
    out.reserve(2 * bytes.len());
    out.extend(
        bytes
            .iter()
            .flat_map(|byte| [digit(byte >> 4), digit(byte & 0x0f)]),
    );
}

const fn digit(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ => b'a' + nibble - 10,
    }
}

/// The bytes of an even-length string of hex digits, either case.
#[must_use]
pub fn unhex(text: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    text.len()
        .is_multiple_of(2)
        .then_some(text)?
        .as_chunks::<2>()
        .0
        .iter()
        .try_fold(
            Zeroizing::new(Vec::with_capacity(text.len() / 2)),
            |out, &[high, low]| {
                let byte = nibble(high)
                    .zip(nibble(low))
                    .map(|(high, low)| (high << 4) | low)?;
                Some(appended(out, |bytes| bytes.push(byte)))
            },
        )
}

const fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

const PASSWORD: &str = "password: ";

/// The stored value from the stderr of `find-generic-password -g`.
#[must_use]
pub fn decode_password(stderr: &[u8]) -> Option<Zeroizing<String>> {
    let line = stderr
        .split(|byte| *byte == b'\n')
        .find_map(|line| line.strip_prefix(PASSWORD.as_bytes()))?;
    let bytes = match (line.strip_prefix(b"0x"), line.strip_prefix(b"\"")) {
        (Some(hex), _) => hex.split(|byte| *byte == b' ').next().and_then(unhex),
        (None, Some(quoted)) => quoted
            .strip_suffix(b"\"")
            .map(|value| Zeroizing::new(value.to_vec())),
        (None, None) => line.is_empty().then(|| Zeroizing::new(Vec::new())),
    }?;
    std::str::from_utf8(&bytes)
        .ok()
        .map(|value| Zeroizing::new(value.to_owned()))
}

/// What an exit code and stderr of a finished run mean; `None` code — a signal.
#[must_use]
pub fn classify_run(code: Option<i32>, stderr: &str) -> SecurityExit {
    match code.map(|code| status(code, stderr)) {
        None => SecurityExit::Error(KeychainError::Killed),
        Some(0) => SecurityExit::Done,
        Some(NOT_FOUND) => SecurityExit::Missing,
        Some(NO_INTERACTION) => SecurityExit::Error(KeychainError::Locked),
        Some(AUTH_FAILED) => SecurityExit::Error(KeychainError::Denied),
        Some(CANCELLED) => SecurityExit::Error(KeychainError::Cancelled),
        Some(status) => SecurityExit::Error(KeychainError::Exited { status }),
    }
}

fn status(code: i32, stderr: &str) -> i32 {
    stderr
        .lines()
        .filter(|line| !line.starts_with(PASSWORD))
        .filter_map(|line| line.rsplit_once(": returned "))
        .find_map(|(_, status)| status.trim().parse().ok())
        .unwrap_or_else(|| from_exit_code(code))
}

const fn from_exit_code(code: i32) -> i32 {
    match code {
        0 => 0,
        44 => NOT_FOUND,
        36 => NO_INTERACTION,
        51 => AUTH_FAILED,
        128 => CANCELLED,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use proptest::{
        collection::vec,
        prelude::{any, prop_assert, prop_assert_eq, proptest},
        test_runner::TestCaseError,
    };
    use rstest::rstest;

    use crate::{
        error::local_data::KeychainError,
        laws::config,
        storage::session_store::security::{
            KeychainItem,
            MAX_LINE,
            PASSWORD,
            add_line,
            classify_run,
            decode_password,
            delete_arguments,
            find_arguments,
            hex_into,
            quote,
            room,
            unhex,
        },
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum SplitState {
        SkipSpace,
        Bare,
        BareEscaped,
        Quoted(char),
        QuotedEscaped(char),
    }

    fn split_line(line: &str) -> Vec<String> {
        let (mut words, current, state) = line.chars().fold(
            (Vec::new(), String::new(), SplitState::SkipSpace),
            |(mut words, mut current, state), c| match (state, c) {
                (SplitState::SkipSpace, c) if space(c) => {
                    (words, current, SplitState::SkipSpace)
                }
                (SplitState::SkipSpace, '"' | '\'') => {
                    (words, current, SplitState::Quoted(c))
                }
                (SplitState::SkipSpace | SplitState::Bare, '\\') => {
                    (words, current, SplitState::BareEscaped)
                }
                (SplitState::Bare, c) if space(c) => {
                    words.push(std::mem::take(&mut current));
                    (words, current, SplitState::SkipSpace)
                }
                (
                    SplitState::SkipSpace | SplitState::Bare | SplitState::BareEscaped,
                    c,
                ) => {
                    current.push(c);
                    (words, current, SplitState::Bare)
                }
                (SplitState::Quoted(quote), '\\') => {
                    (words, current, SplitState::QuotedEscaped(quote))
                }
                (SplitState::Quoted(quote), c) if c == quote => {
                    words.push(std::mem::take(&mut current));
                    (words, current, SplitState::SkipSpace)
                }
                (SplitState::Quoted(quote) | SplitState::QuotedEscaped(quote), c) => {
                    current.push(c);
                    (words, current, SplitState::Quoted(quote))
                }
            },
        );
        match state {
            SplitState::SkipSpace => words,
            SplitState::Bare
            | SplitState::BareEscaped
            | SplitState::Quoted(_)
            | SplitState::QuotedEscaped(_) => {
                words.push(current);
                words
            }
        }
    }

    const fn space(c: char) -> bool {
        matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
    }

    const fn printable(byte: u8) -> bool {
        matches!(byte, b' '..=b'~')
    }

    fn printed(bytes: &[u8]) -> Vec<u8> {
        let quoted: Vec<u8> = std::iter::once(b'"')
            .chain(bytes.iter().flat_map(|&byte| match byte {
                b'\\' => b"\\134".to_vec(),
                byte if printable(byte) => vec![byte],
                byte => format!("\\{byte:03o}").into_bytes(),
            }))
            .chain(std::iter::once(b'"'))
            .collect();
        let digits = hexed(Vec::new(), bytes).to_ascii_uppercase();
        let hex = [b"0x".as_slice(), &digits].concat();
        let plain = bytes
            .iter()
            .all(|&byte| printable(byte) && !matches!(byte, b'"' | b'\\'));
        let shown = match (bytes.is_empty(), plain) {
            (true, _) => Vec::new(),
            (false, true) => quoted,
            (false, false) => [hex, b"  ".to_vec(), quoted].concat(),
        };
        [PASSWORD.as_bytes(), &shown, b"\n"].concat()
    }

    fn hexed(mut out: Vec<u8>, bytes: &[u8]) -> Vec<u8> {
        hex_into(&mut out, bytes);
        out
    }

    fn name() -> impl proptest::strategy::Strategy<Value = String> {
        "[^\n\u{0}]{0,40}"
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn hex_round_trips(bytes in vec(any::<u8>(), 0..512)) {
            let out = hexed(Vec::new(), &bytes);
            prop_assert_eq!(out.len(), 2 * bytes.len());
            prop_assert!(out.iter().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            prop_assert_eq!(unhex(&out).map(|b| b.to_vec()), Some(bytes));
        }

        #[test]
        fn quoted_words_split_back(words in vec(name(), 1..6)) {
            let line = words
                .iter()
                .filter_map(|word| quote(word))
                .collect::<Vec<_>>()
                .join(" ");
            prop_assert_eq!(split_line(&line), words);
        }

        #[test]
        fn add_line_carries_the_secret_only_as_hex(
            secret in vec(any::<u8>(), 0..1500),
            service in name(),
            account in name(),
            keychain in proptest::option::of(name()),
        ) {
            let item = KeychainItem { service: &service, account: &account, keychain: keychain.as_deref() };
            let line = add_line(&item, &secret).map_err(|err| TestCaseError::fail(err.to_string()))?;
            let text = String::from_utf8(line.to_vec()).unwrap_or_default();
            prop_assert!(line.len() <= MAX_LINE + 1 && text.ends_with('\n'));
            let words = split_line(text.trim_end_matches('\n'));
            let hex = String::from_utf8(hexed(Vec::new(), &secret)).unwrap_or_default();
            let expected: Vec<String> = [
                "add-generic-password".to_owned(), "-U".into(), "-s".into(), service.clone(),
                "-a".into(), account.clone(), "-X".into(), hex,
            ]
            .into_iter()
            .chain(keychain.clone())
            .collect();
            prop_assert_eq!(words, expected);
        }

        #[test]
        fn add_line_refuses_a_secret_the_line_cannot_carry(
            secret in vec(any::<u8>(), MAX_LINE..MAX_LINE + 500),
            service in name(),
            account in name(),
            keychain in proptest::option::of(name()),
        ) {
            let item = KeychainItem { service: &service, account: &account, keychain: keychain.as_deref() };
            let max = room(&item).map_err(|err| TestCaseError::fail(err.to_string()))?;
            let refused = add_line(&item, &secret).err().map(|err| err.to_string());
            prop_assert_eq!(
                refused,
                Some(KeychainError::TooLong { len: secret.len(), max }.to_string())
            );
        }

        #[test]
        fn printed_value_reads_back(value in any::<String>(), hex_digits in "[0-9a-fA-F]{0,64}") {
            [value, hex_digits].into_iter().try_for_each(|value| {
                let decoded = decode_password(&printed(value.as_bytes())).map(|v| v.to_string());
                prop_assert_eq!(decoded, Some(value));
                Ok(())
            })?;
        }
    }

    #[rstest]
    #[case::bare_token(b"password: \"aaaa\"\n", "aaaa")]
    #[case::hex_of_printable(b"password: \"4142\"\n", "4142")]
    #[case::dashless_uuid(
        b"password: \"0b2f7c8e1d3a4c5b9e6f7a8b9c0d1e2f\"\n",
        "0b2f7c8e1d3a4c5b9e6f7a8b9c0d1e2f"
    )]
    #[case::json(
        b"password: 0x7B22746F6B656E223A2274227D  \"{\"token\":\"t\"}\"\n",
        "{\"token\":\"t\"}"
    )]
    #[case::utf8(b"password: 0x68C3A96C6C6F  \"h\\303\\251llo\"\n", "h\u{e9}llo")]
    #[case::newline(b"password: 0x610A62  \"a\\012b\"\n", "a\nb")]
    #[case::empty(b"password: \n", "")]
    #[case::after_other_lines(b"note\npassword: \"abc\"", "abc")]
    fn decodes_what_security_prints(#[case] stderr: &[u8], #[case] value: &str) {
        assert_eq!(
            decode_password(stderr).map(|v| v.to_string()).as_deref(),
            Some(value)
        );
    }

    #[rstest]
    #[case::not_utf8(b"password: 0xFF00  \"\\377\\000\"\n")]
    #[case::no_line(b"security: could not be found\n")]
    #[case::unclosed(b"password: \"abc\n")]
    fn unreadable_value_is_none(#[case] stderr: &[u8]) {
        assert!(decode_password(stderr).is_none());
    }

    #[rstest]
    #[case::done(Some(0), "", "Done")]
    #[case::missing_by_code(Some(44), "security: ... could not be found", "Missing")]
    #[case::missing_by_line(
        Some(44),
        "find-generic-password: returned -25300\n",
        "Missing"
    )]
    #[case::locked(Some(36), "", "Error(Locked)")]
    #[case::denied(Some(51), "", "Error(Denied)")]
    #[case::cancelled(Some(128), "", "Error(Cancelled)")]
    #[case::cancelled_by_line(
        Some(128),
        "add-generic-password: returned -128\n",
        "Error(Cancelled)"
    )]
    #[case::other(
        Some(1),
        "security: unknown command \"x\"\n",
        "Error(Exited { status: 1 })"
    )]
    #[case::other_line(
        Some(50),
        "x: returned -25294\n",
        "Error(Exited { status: -25294 })"
    )]
    #[case::killed(None, "", "Error(Killed)")]
    fn exit_codes_map_to_verdicts(
        #[case] code: Option<i32>,
        #[case] stderr: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(format!("{:?}", classify_run(code, stderr)), expected);
    }

    #[test]
    fn names_with_line_breaks_are_refused() {
        let item = KeychainItem {
            service: "a\nb",
            account: "c",
            keychain: None,
        };
        assert!(matches!(add_line(&item, b"x"), Err(KeychainError::BadName)));
        assert_eq!(quote("a\0"), None);
    }

    #[test]
    fn argv_never_holds_a_value() {
        let item = KeychainItem {
            service: "s",
            account: "a",
            keychain: Some("/k"),
        };
        assert_eq!(
            find_arguments(&item),
            ["find-generic-password", "-s", "s", "-a", "a", "-g", "/k"]
        );
        assert_eq!(
            delete_arguments(&KeychainItem {
                keychain: None,
                ..item
            }),
            ["delete-generic-password", "-s", "s", "-a", "a"]
        );
    }
}
