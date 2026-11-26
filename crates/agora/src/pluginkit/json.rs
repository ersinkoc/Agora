//! JSON read and written as `JSON.parse` and `JSON.stringify(value, null, 2)` do, for a plugin's manifest (`agora plugin pack
//! --write-hashes` rewrites it as `scripts/pack-plugin.mjs` does, byte for byte). serde_json's map sorts its keys, and turning on its
//! `preserve_order` feature would change every map of the binary, so the structure is read here and only strings and numbers are left to
//! serde_json and `str::parse`:
//! - an object keeps its keys in the order written; a key given twice keeps its first place and its last value;
//! - keys come out in JavaScript's order: array indexes (`"0"`, `"17"`) first, ascending, then the others in their order;
//! - a number is a double, written as JavaScript writes it (`1.0` is `1`, `1e21` is `1e+21`, an infinite one is `null`);
//! - at most 64 objects and arrays open at once, as the window and the hub read a package.
//!
//! A string with a lone surrogate (`"\ud800"`) is not read; no manifest the window accepts holds one.

/// The most objects and arrays open at once (the window's `MAX_JSON_DEPTH`).
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Val>),
    Obj(Obj),
}

/// An object's members in the order they were first written, each key once.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Obj(Vec<(String, Val)>);

impl Obj {
    pub(crate) fn get(&self, key: &str) -> Option<&Val> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Sets a member as JavaScript's assignment does: a key already there keeps its place.
    pub(crate) fn set(&mut self, key: &str, value: Val) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    /// The members in JavaScript's order (`Object.keys`).
    pub(crate) fn members(&self) -> Vec<&(String, Val)> {
        let mut indexes: Vec<(u32, &(String, Val))> = self
            .0
            .iter()
            .filter_map(|m| array_index(&m.0).map(|i| (i, m)))
            .collect();
        indexes.sort_by_key(|(i, _)| *i);
        let mut out: Vec<&(String, Val)> = indexes.into_iter().map(|(_, m)| m).collect();
        out.extend(self.0.iter().filter(|m| array_index(&m.0).is_none()));
        out
    }

    pub(crate) fn keys(&self) -> Vec<&str> {
        self.members()
            .into_iter()
            .map(|(k, _)| k.as_str())
            .collect()
    }
}

/// A key JavaScript treats as an array index: a canonical decimal below 2^32 - 1.
pub(crate) fn array_index(k: &str) -> Option<u32> {
    if k.is_empty()
        || k.len() > 10
        || !k.bytes().all(|b| b.is_ascii_digit())
        || (k.len() > 1 && k.starts_with('0'))
    {
        return None;
    }
    k.parse::<u64>()
        .ok()
        .filter(|n| *n < u64::from(u32::MAX))
        .and_then(|n| u32::try_from(n).ok())
}

/// Reads UTF-8 JSON text (no byte-order mark, as Node's `readFileSync(…, 'utf8')` and `JSON.parse` refuse one), or `None`.
pub(crate) fn parse(bytes: &[u8]) -> Option<Val> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
    };
    p.space();
    let v = p.value(0)?;
    p.space();
    (p.i == p.s.len()).then_some(v)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, word: &[u8]) -> Option<()> {
        let end = self.i.checked_add(word.len())?;
        (self.s.get(self.i..end)? == word).then(|| self.i = end)
    }

    fn value(&mut self, depth: usize) -> Option<Val> {
        match self.peek()? {
            b'{' => self.object(depth + 1),
            b'[' => self.array(depth + 1),
            b'"' => self.string().map(Val::Str),
            b't' => self.eat(b"true").map(|()| Val::Bool(true)),
            b'f' => self.eat(b"false").map(|()| Val::Bool(false)),
            b'n' => self.eat(b"null").map(|()| Val::Null),
            b'-' | b'0'..=b'9' => self.number().map(Val::Num),
            _ => None,
        }
    }

    fn object(&mut self, depth: usize) -> Option<Val> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.i += 1;
        let mut obj = Obj::default();
        self.space();
        if self.peek()? == b'}' {
            self.i += 1;
            return Some(Val::Obj(obj));
        }
        loop {
            self.space();
            if self.peek()? != b'"' {
                return None;
            }
            let key = self.string()?;
            self.space();
            self.eat(b":")?;
            self.space();
            let v = self.value(depth)?;
            obj.set(&key, v);
            self.space();
            match self.peek()? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Some(Val::Obj(obj));
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self, depth: usize) -> Option<Val> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.i += 1;
        let mut out = Vec::new();
        self.space();
        if self.peek()? == b']' {
            self.i += 1;
            return Some(Val::Arr(out));
        }
        loop {
            self.space();
            out.push(self.value(depth)?);
            self.space();
            match self.peek()? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Some(Val::Arr(out));
                }
                _ => return None,
            }
        }
    }

    /// A string token, its escapes read by serde_json (which refuses a control character, a bad escape and a lone surrogate).
    fn string(&mut self) -> Option<String> {
        let start = self.i;
        self.i += 1;
        loop {
            match self.peek()? {
                b'"' => break,
                b'\\' => self.i += 2,
                _ => self.i += 1,
            }
        }
        self.i += 1;
        serde_json::from_str::<String>(self.text.get(start..self.i)?).ok()
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`, read as a double.
    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek()? {
            b'0' => self.i += 1,
            b'1'..=b'9' => self.digits(),
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return None;
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return None;
            }
            self.digits();
        }
        self.text.get(start..self.i)?.parse::<f64>().ok()
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
    }
}

/// `JSON.stringify(value, null, 2)`.
pub(crate) fn stringify_pretty(v: &Val) -> String {
    let mut out = String::new();
    write(&mut out, v, 0);
    out
}

fn write(out: &mut String, v: &Val, indent: usize) {
    match v {
        Val::Null => out.push_str("null"),
        Val::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Val::Num(n) => out.push_str(&js_number(*n)),
        Val::Str(s) => out.push_str(&quote(s)),
        Val::Arr(items) if items.is_empty() => out.push_str("[]"),
        Val::Obj(o) if o.0.is_empty() => out.push_str("{}"),
        Val::Arr(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                out.push_str(if n == 0 { "\n" } else { ",\n" });
                pad(out, indent + 1);
                write(out, item, indent + 1);
            }
            out.push('\n');
            pad(out, indent);
            out.push(']');
        }
        Val::Obj(o) => {
            out.push('{');
            for (n, (k, item)) in o.members().into_iter().enumerate() {
                out.push_str(if n == 0 { "\n" } else { ",\n" });
                pad(out, indent + 1);
                out.push_str(&quote(k));
                out.push_str(": ");
                write(out, item, indent + 1);
            }
            out.push('\n');
            pad(out, indent);
            out.push('}');
        }
    }
}

fn pad(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// A string as `JSON.stringify` writes it: serde_json escapes the same characters (`"`, `\`, and the controls below U+0020 as
/// `\b \t \n \f \r` or `\u00xx` in lower case) and leaves the rest as it is.
pub(crate) fn quote(s: &str) -> String {
    // A `&str` always serialises (no map keys, no I/O).
    serde_json::to_string(s).unwrap_or_default()
}

/// A double as JavaScript's `Number.prototype.toString` writes it (ECMA-262 `Number::toString`), from the shortest digits that read
/// back as the same double (which Rust's `{:e}` gives).
pub(crate) fn js_number(x: f64) -> String {
    if !x.is_finite() {
        return "null".to_string();
    }
    if x == 0.0 {
        return "0".to_string();
    }
    if x < 0.0 {
        return format!("-{}", js_number(-x));
    }
    let e = format!("{x:e}");
    let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i64;
    let n = exp.parse::<i64>().unwrap_or(0) + 1;
    let zeros = |count: i64| "0".repeat(usize::try_from(count).unwrap_or(0));
    if k <= n && n <= 21 {
        format!("{digits}{}", zeros(n - k))
    } else if 0 < n && n <= 21 {
        let (a, b) = digits.split_at(usize::try_from(n).unwrap_or(0));
        format!("{a}.{b}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", zeros(-n))
    } else {
        let (first, rest) = digits.split_at(1);
        let dot = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let sign = if n > 0 { "+" } else { "-" };
        format!("{first}{dot}e{sign}{}", (n - 1).abs())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn obj(text: &str) -> Obj {
        match parse(text.as_bytes()).unwrap() {
            Val::Obj(o) => o,
            v => panic!("not an object: {v:?}"),
        }
    }

    #[test]
    fn keeps_keys_in_javascript_order() {
        let o = obj(r#"{"b":1,"a":2,"10":3,"2":4,"01":5,"b":6}"#);
        assert_eq!(o.keys(), ["2", "10", "b", "a", "01"]);
        assert_eq!(o.get("b"), Some(&Val::Num(6.0)));
        assert_eq!(array_index("4294967294"), Some(4_294_967_294));
        assert_eq!(array_index("4294967295"), None);
    }

    #[test]
    fn reads_only_json() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "-",
            "+1",
            "tru",
            "\"\\x\"",
            "\"\u{1}\"",
            "\u{feff}{}",
            "{} x",
            "\"\\ud800\"",
            "NaN",
        ] {
            assert!(parse(bad.as_bytes()).is_none(), "{bad:?}");
        }
        assert!(parse(&[b'"', 0xff, b'"']).is_none());
        let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(parse(deep(64).as_bytes()).is_some());
        assert!(parse(deep(65).as_bytes()).is_none());
    }

    #[test]
    fn writes_as_json_stringify_does() {
        let v = parse(r#"{"s":"a\"b\\c\n\u0001\u2028é😀/","n":[1.0,-0,1e21,1.5e-7,123456789012345680000,0.000001,1e400],"e":{},"l":[],"t":true,"z":null}"#.as_bytes()).unwrap();
        assert_eq!(
            stringify_pretty(&v),
            "{\n  \"s\": \"a\\\"b\\\\c\\n\\u0001\u{2028}é😀/\",\n  \"n\": [\n    1,\n    0,\n    1e+21,\n    1.5e-7,\n    123456789012345680000,\n    0.000001,\n    null\n  ],\n  \"e\": {},\n  \"l\": [],\n  \"t\": true,\n  \"z\": null\n}"
        );
        assert_eq!(js_number(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(js_number(-2.5e-10), "-2.5e-10");
        assert_eq!(js_number(100.0), "100");
    }
}
