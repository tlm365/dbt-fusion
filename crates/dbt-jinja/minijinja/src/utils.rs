use std::char::decode_utf16;
use std::collections::BTreeMap;
use std::fmt;
use std::iter::{once, repeat};
use std::str::Chars;

use crate::error::{Error, ErrorKind};
use crate::value::{StringType, Value, ValueIter, ValueKind, ValueRepr};
use crate::Output;

/// internal marker to seal up some trait methods
pub struct SealedMarker;

pub fn memchr(haystack: &[u8], needle: u8) -> Option<usize> {
    haystack.iter().position(|&x| x == needle)
}

pub fn memstr(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Helper for dealing with untrusted size hints.
#[inline(always)]
pub(crate) fn untrusted_size_hint(value: usize) -> usize {
    value.min(1024)
}

fn write_with_html_escaping(out: &mut Output, value: &Value) -> fmt::Result {
    if matches!(
        value.kind(),
        ValueKind::Undefined | ValueKind::None | ValueKind::Bool | ValueKind::Number
    ) {
        write!(out, "{value}")
    } else if let Some(s) = value.as_str() {
        write!(out, "{}", HtmlEscape(s))
    } else {
        write!(out, "{}", HtmlEscape(&value.to_string()))
    }
}

fn invalid_autoescape(name: &str) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::InvalidOperation,
        format!("Default formatter does not know how to format to custom format '{name}'"),
    ))
}

#[inline(always)]
pub fn write_escaped(
    out: &mut Output,
    auto_escape: AutoEscape,
    value: &Value,
) -> Result<(), Error> {
    // common case of safe strings or strings without auto escaping
    if let ValueRepr::String(ref s, ty) = value.0 {
        if matches!(ty, StringType::Safe) || matches!(auto_escape, AutoEscape::None) {
            return out.write_str(s).map_err(Error::from);
        }
    }

    match auto_escape {
        AutoEscape::None => write!(out, "{value}").map_err(Error::from),
        AutoEscape::Html => write_with_html_escaping(out, value).map_err(Error::from),
        #[cfg(feature = "json")]
        AutoEscape::Json => {
            let value = ok!(serde_json::to_string(&value).map_err(|err| {
                Error::new(ErrorKind::BadSerialization, "unable to format to JSON").with_source(err)
            }));
            write!(out, "{value}").map_err(Error::from)
        }
        AutoEscape::Custom(name) => invalid_autoescape(name),
    }
}

/// Controls the autoescaping behavior.
///
/// For more information see
/// [`set_auto_escape_callback`](crate::Environment::set_auto_escape_callback).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AutoEscape {
    /// Do not apply auto escaping.
    None,
    /// Use HTML auto escaping rules.
    ///
    /// Any value will be converted into a string and the following characters
    /// will be escaped in ways compatible to XML and HTML: `<`, `>`, `&`, `"`,
    /// `'`, and `/`.
    Html,
    /// Use escaping rules suitable for JSON/JavaScript or YAML.
    ///
    /// Any value effectively ends up being serialized to JSON upon printing.  The
    /// serialized values will be compatible with JavaScript and YAML as well.
    #[cfg(feature = "json")]
    #[cfg_attr(docsrs, doc(cfg(feature = "json")))]
    Json,
    /// A custom auto escape format.
    ///
    /// The default formatter does not know how to deal with a custom escaping
    /// format and would error.  The use of these requires a custom formatter.
    /// See [`set_formatter`](crate::Environment::set_formatter).
    Custom(&'static str),
}

/// Defines the behavior of undefined values in the engine.
///
/// At present there are three types of behaviors available which mirror the behaviors
/// that Jinja2 provides out of the box.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum UndefinedBehavior {
    /// The default, somewhat lenient undefined behavior.
    ///
    /// * **printing:** allowed (returns empty string)
    /// * **iteration:** allowed (returns empty array)
    /// * **attribute access of undefined values:** fails
    #[default]
    Lenient,
    /// Like `Lenient`, but also allows chaining of undefined lookups.
    ///
    /// * **printing:** allowed (returns empty string)
    /// * **iteration:** allowed (returns empty array)
    /// * **attribute access of undefined values:** allowed (returns [`undefined`](Value::UNDEFINED))
    Chainable,
    /// Like `Chainable`, but also allows calling methods on undefined values
    ///
    /// * **printing:** allowed (returns empty string)
    /// * **iteration:** allowed (returns empty array)
    /// * **attribute access of undefined values:** allowed (returns [`undefined`](Value::UNDEFINED))
    /// * **method calls on undefined values:** allowed (returns [`undefined`](Value::UNDEFINED))
    /// * **method calls on None values:** allowed (returns [`undefined`](Value::UNDEFINED))
    Dbt,
    /// Complains very quickly about undefined values.
    ///
    /// * **printing:** fails
    /// * **iteration:** fails
    /// * **attribute access of undefined values:** fails
    Strict,
}

impl UndefinedBehavior {
    /// Utility method used in the engine to determine what to do when an undefined is
    /// encountered.
    ///
    /// The flag indicates if this is the first or second level of undefined value.  If
    /// `parent_was_undefined` is set to `true`, the undefined was created by looking up
    /// a missing attribute on an undefined value.  If `false` the undefined was created by
    /// looking up a missing attribute on a defined value.
    pub fn handle_undefined(self, parent_was_undefined: Option<bool>) -> Result<Value, Error> {
        match (self, parent_was_undefined) {
            (UndefinedBehavior::Dbt, _) => Ok(Value::UNDEFINED),
            (UndefinedBehavior::Lenient, Some(false))
            | (UndefinedBehavior::Lenient, None)
            | (UndefinedBehavior::Strict, Some(false))
            | (UndefinedBehavior::Chainable, _) => Ok(Value::UNDEFINED),
            (UndefinedBehavior::Lenient, Some(true)) | (UndefinedBehavior::Strict, _) => {
                Err(Error::from(ErrorKind::UndefinedError))
            }
        }
    }

    /// Utility method to optionally return undefined values when the caller is None.
    pub fn handle_undefined_none(self, method: &str) -> Result<Value, Error> {
        match self {
            UndefinedBehavior::Dbt => Ok(Value::UNDEFINED),
            _ => Err(Error::from(ErrorKind::UnknownMethod(
                "None".to_string(),
                method.to_string(),
            ))),
        }
    }

    /// Utility method to check if something is true.
    ///
    /// This fails only for strict undefined values.
    #[inline]
    pub(crate) fn is_true(self, value: &Value) -> Result<bool, Error> {
        if matches!(self, UndefinedBehavior::Strict) && value.is_undefined() {
            Err(Error::from(ErrorKind::UndefinedError))
        } else {
            Ok(value.is_true())
        }
    }

    /// Tries to iterate over a value while handling the undefined value.
    ///
    /// If the value is undefined, then iteration fails if the behavior is set to strict,
    /// otherwise it succeeds with an empty iteration.  This is also internally used in the
    /// engine to convert values to lists.
    #[inline]
    pub(crate) fn try_iter(self, value: Value) -> Result<ValueIter, Error> {
        self.assert_iterable(&value).and_then(|_| value.try_iter())
    }

    /// Are we strict on iteration?
    #[inline]
    pub(crate) fn assert_iterable(self, value: &Value) -> Result<(), Error> {
        if matches!(self, UndefinedBehavior::Strict) && value.is_undefined() {
            Err(Error::from(ErrorKind::UndefinedError))
        } else {
            Ok(())
        }
    }
}

/// Helper to HTML escape a string.
pub struct HtmlEscape<'a>(pub &'a str);

impl fmt::Display for HtmlEscape<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "v_htmlescape")]
        {
            fmt::Display::fmt(&v_htmlescape::escape(self.0), f)
        }
        // this is taken from askama-escape
        #[cfg(not(feature = "v_htmlescape"))]
        {
            let bytes = self.0.as_bytes();
            let mut start = 0;

            for (i, b) in bytes.iter().enumerate() {
                macro_rules! escaping_body {
                    ($quote:expr) => {{
                        if start < i {
                            // SAFETY: this is safe because we only push valid utf-8 bytes over
                            ok!(f.write_str(unsafe {
                                std::str::from_utf8_unchecked(&bytes[start..i])
                            }));
                        }
                        ok!(f.write_str($quote));
                        start = i + 1;
                    }};
                }
                if b.wrapping_sub(b'"') <= b'>' - b'"' {
                    match *b {
                        b'<' => escaping_body!("&lt;"),
                        b'>' => escaping_body!("&gt;"),
                        b'&' => escaping_body!("&amp;"),
                        b'"' => escaping_body!("&quot;"),
                        b'\'' => escaping_body!("&#x27;"),
                        b'/' => escaping_body!("&#x2f;"),
                        _ => (),
                    }
                }
            }

            if start < bytes.len() {
                // SAFETY: this is safe because we only push valid utf-8 bytes over
                f.write_str(unsafe { std::str::from_utf8_unchecked(&bytes[start..]) })
            } else {
                Ok(())
            }
        }
    }
}

struct Unescaper {
    out: String,
    pending_surrogate: u16,
}

impl Unescaper {
    fn unescape(mut self, s: &str) -> Result<String, Error> {
        let mut char_iter = s.chars();

        while let Some(c) = char_iter.next() {
            if c == '\\' {
                match char_iter.next() {
                    None => return Err(ErrorKind::BadEscape.into()),
                    Some(d) => match d {
                        '"' | '\\' | '/' | '\'' => ok!(self.push_char(d)),
                        'b' => ok!(self.push_char('\x08')),
                        'f' => ok!(self.push_char('\x0C')),
                        'n' => ok!(self.push_char('\n')),
                        'r' => ok!(self.push_char('\r')),
                        't' => ok!(self.push_char('\t')),
                        'u' => {
                            let val = ok!(self.parse_u16(&mut char_iter));
                            ok!(self.push_u16(val));
                        }
                        '\n' => ok!(self.push_char('\n')),
                        '\r' => ok!(self.push_char('\r')),
                        // Removing this line because we should not error out on unknown escapes
                        // In python jinja, json escaping is not strictly enforced
                        // _ => return Err(ErrorKind::BadEscape.into()),
                        _ => {
                            ok!(self.push_char('\\'));
                            ok!(self.push_char(d));
                        }
                    },
                }
            } else {
                ok!(self.push_char(c));
            }
        }

        if self.pending_surrogate != 0 {
            Err(ErrorKind::BadEscape.into())
        } else {
            Ok(self.out)
        }
    }

    fn parse_u16(&self, chars: &mut Chars) -> Result<u16, Error> {
        let hexnum = chars.chain(repeat('\0')).take(4).collect::<String>();
        u16::from_str_radix(&hexnum, 16).map_err(|_| ErrorKind::BadEscape.into())
    }

    fn push_u16(&mut self, c: u16) -> Result<(), Error> {
        match (self.pending_surrogate, (0xD800..=0xDFFF).contains(&c)) {
            (0, false) => match decode_utf16(once(c)).next() {
                Some(Ok(c)) => self.out.push(c),
                _ => return Err(ErrorKind::BadEscape.into()),
            },
            (_, false) => return Err(ErrorKind::BadEscape.into()),
            (0, true) => self.pending_surrogate = c,
            (prev, true) => match decode_utf16(once(prev).chain(once(c))).next() {
                Some(Ok(c)) => {
                    self.out.push(c);
                    self.pending_surrogate = 0;
                }
                _ => return Err(ErrorKind::BadEscape.into()),
            },
        }
        Ok(())
    }

    fn push_char(&mut self, c: char) -> Result<(), Error> {
        if self.pending_surrogate != 0 {
            Err(ErrorKind::BadEscape.into())
        } else {
            self.out.push(c);
            Ok(())
        }
    }
}

/// Un-escape a string, following JSON rules.
pub fn unescape(s: &str) -> Result<String, Error> {
    Unescaper {
        out: String::new(),
        pending_surrogate: 0,
    }
    .unescape(s)
}

pub struct BTreeMapKeysDebug<'a, K: fmt::Debug, V>(pub &'a BTreeMap<K, V>);

impl<K: fmt::Debug, V> fmt::Debug for BTreeMapKeysDebug<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.iter().map(|x| x.0)).finish()
    }
}

pub struct OnDrop<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> OnDrop<F> {
    pub fn new(f: F) -> Self {
        Self(Some(f))
    }
}

impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        self.0.take().unwrap()();
    }
}

#[cfg(feature = "builtins")]
pub fn splitn_whitespace(s: &str, maxsplits: usize) -> impl Iterator<Item = &str> + '_ {
    let mut splits = 1;
    let mut skip_ws = true;
    let mut split_start = None;
    let mut last_split_end = 0;
    let mut chars = s.char_indices();

    std::iter::from_fn(move || {
        for (idx, c) in chars.by_ref() {
            if splits >= maxsplits && !skip_ws {
                continue;
            } else if c.is_whitespace() {
                if let Some(old) = split_start {
                    let rv = &s[old..idx];
                    split_start = None;
                    last_split_end = idx;
                    splits += 1;
                    skip_ws = true;
                    return Some(rv);
                }
            } else {
                skip_ws = false;
                if split_start.is_none() {
                    split_start = Some(idx);
                    last_split_end = idx;
                }
            }
        }

        let rest = &s[last_split_end..];
        if !rest.is_empty() {
            last_split_end = s.len();
            Some(rest)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use similar_asserts::assert_eq;

    #[test]
    fn test_html_escape() {
        let input = "<>&\"'/";
        let output = HtmlEscape(input).to_string();
        assert_eq!(output, "&lt;&gt;&amp;&quot;&#x27;&#x2f;");
    }

    #[test]
    fn test_unescape() {
        assert_eq!(unescape(r"foo\u2603bar").unwrap(), "foo\u{2603}bar");
        assert_eq!(unescape(r"\t\b\f\r\n\\\/").unwrap(), "\t\x08\x0c\r\n\\/");
        assert_eq!(unescape("foobarbaz").unwrap(), "foobarbaz");
        assert_eq!(unescape(r"\ud83d\udca9").unwrap(), "💩");
    }

    #[test]
    #[cfg(feature = "builtins")]
    fn test_splitn_whitespace() {
        fn s(s: &str, n: usize) -> Vec<&str> {
            splitn_whitespace(s, n).collect::<Vec<_>>()
        }

        assert_eq!(s("a b c", 1), vec!["a b c"]);
        assert_eq!(s("a b c", 2), vec!["a", "b c"]);
        assert_eq!(s("a    b c", 2), vec!["a", "b c"]);
        assert_eq!(s("a    b c   ", 2), vec!["a", "b c   "]);
        assert_eq!(s("a   b   c", 3), vec!["a", "b", "c"]);
        assert_eq!(s("a   b   c", 4), vec!["a", "b", "c"]);
        assert_eq!(s("   a   b   c", 3), vec!["a", "b", "c"]);
        assert_eq!(s("   a   b   c", 4), vec!["a", "b", "c"]);
    }
}
