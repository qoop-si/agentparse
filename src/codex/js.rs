//! A tolerant reader for the program a code-mode cell runs. It understands one shape, a list of
//! `text(await tools.NAME(ARG));` statements whose `ARG` is a literal (string, number, `true`,
//! `false`, `null`, array, object). Any other text makes the whole program unreadable. Nothing
//! is evaluated.

use serde_json::{Map, Number, Value};

/// The tool calls of `source` in order, or `None` when it has any other statement.
pub(super) fn invocations(source: &str) -> Option<Vec<(String, Value)>> {
    let mut r = Reader {
        c: source.chars().collect(),
        i: 0,
    };
    let mut calls = Vec::new();
    loop {
        while r.eat(';').is_some() {}
        r.skip_space();
        if r.i >= r.c.len() {
            return Some(calls);
        }
        r.word("text")?;
        r.eat('(')?;
        r.word("await")?;
        r.word("tools")?;
        r.eat('.')?;
        let name = r.ident()?;
        r.eat('(')?;
        let arg = r.value(0)?;
        r.eat(')')?;
        r.eat(')')?;
        calls.push((name, arg));
    }
}

struct Reader {
    c: Vec<char>,
    i: usize,
}

impl Reader {
    fn skip_space(&mut self) {
        while self.c.get(self.i).is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn eat(&mut self, ch: char) -> Option<()> {
        self.skip_space();
        (self.c.get(self.i) == Some(&ch)).then(|| self.i += 1)
    }

    fn ident(&mut self) -> Option<String> {
        self.skip_space();
        let start = self.i;
        while (self.c.get(self.i)).is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$')) {
            self.i += 1;
        }
        (self.i > start).then(|| self.c[start..self.i].iter().collect())
    }

    fn word(&mut self, word: &str) -> Option<()> {
        (self.ident()? == word).then_some(())
    }

    fn value(&mut self, depth: u8) -> Option<Value> {
        if depth > 32 {
            return None;
        }
        self.skip_space();
        match *self.c.get(self.i)? {
            '"' | '\'' | '`' => self.string().map(Value::String),
            '{' => self.object(depth),
            '[' => self.array(depth),
            '-' | '0'..='9' => self.number(),
            _ => match self.ident()?.as_str() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                "null" => Some(Value::Null),
                _ => None,
            },
        }
    }

    fn object(&mut self, depth: u8) -> Option<Value> {
        self.eat('{')?;
        let mut map = Map::new();
        while self.eat('}').is_none() {
            self.skip_space();
            let key = match *self.c.get(self.i)? {
                '"' | '\'' => self.string()?,
                _ => self.ident()?,
            };
            self.eat(':')?;
            map.insert(key, self.value(depth + 1)?);
            if self.eat(',').is_none() {
                self.eat('}')?;
                break;
            }
        }
        Some(Value::Object(map))
    }

    fn array(&mut self, depth: u8) -> Option<Value> {
        self.eat('[')?;
        let mut items = Vec::new();
        while self.eat(']').is_none() {
            items.push(self.value(depth + 1)?);
            if self.eat(',').is_none() {
                self.eat(']')?;
                break;
            }
        }
        Some(Value::Array(items))
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.i;
        while (self.c.get(self.i)).is_some_and(|c| c.is_ascii_digit() || "+-.eE".contains(*c)) {
            self.i += 1;
        }
        let text: String = self.c[start..self.i].iter().collect();
        match text.parse::<i64>() {
            Ok(n) => Some(n.into()),
            Err(_) => Number::from_f64(text.parse().ok()?).map(Value::Number),
        }
    }

    /// A quoted string with JavaScript escapes. A template literal with `${` is not read.
    fn string(&mut self) -> Option<String> {
        let quote = *self.c.get(self.i)?;
        self.i += 1;
        let mut out = String::new();
        loop {
            let ch = *self.c.get(self.i)?;
            self.i += 1;
            match ch {
                c if c == quote => return Some(out),
                '$' if quote == '`' && self.c.get(self.i) == Some(&'{') => return None,
                '\\' => out.extend(self.escape()?),
                c => out.push(c),
            }
        }
    }

    fn escape(&mut self) -> Option<Option<char>> {
        let ch = *self.c.get(self.i)?;
        self.i += 1;
        Some(Some(match ch {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'v' => '\u{b}',
            '0' => '\0',
            'x' => self.hex(2)?,
            'u' => self.unicode()?,
            // A backslash before a line break continues the string.
            '\n' | '\u{2028}' | '\u{2029}' => return Some(None),
            '\r' => {
                if self.c.get(self.i) == Some(&'\n') {
                    self.i += 1;
                }
                return Some(None);
            }
            c => c,
        }))
    }

    fn hex(&mut self, n: usize) -> Option<char> {
        let digits: String = self.c.get(self.i..self.i + n)?.iter().collect();
        self.i += n;
        char::from_u32(u32::from_str_radix(&digits, 16).ok()?)
    }

    /// `\uXXXX` (a surrogate pair is joined) or `\u{X...}`.
    fn unicode(&mut self) -> Option<char> {
        if self.c.get(self.i) == Some(&'{') {
            let end = self.c[self.i..].iter().position(|c| *c == '}')? + self.i;
            let digits: String = self.c[self.i + 1..end].iter().collect();
            self.i = end + 1;
            return char::from_u32(u32::from_str_radix(&digits, 16).ok()?);
        }
        let unit = |r: &mut Reader| {
            let digits: String = r.c.get(r.i..r.i + 4)?.iter().collect();
            r.i += 4;
            u32::from_str_radix(&digits, 16).ok()
        };
        let high = unit(self)?;
        if (0xD800..0xDC00).contains(&high)
            && self.c.get(self.i) == Some(&'\\')
            && self.c.get(self.i + 1) == Some(&'u')
        {
            let at = self.i;
            self.i += 2;
            match unit(self) {
                Some(low @ 0xDC00..=0xDFFF) => {
                    return char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00));
                }
                // Not a pair: the lone surrogate is U+FFFD and the next escape reads on its own.
                _ => self.i = at,
            }
        }
        Some(char::from_u32(high).unwrap_or('\u{FFFD}'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_statements_in_order() {
        let src = "text(await tools.exec_command({cmd:\"ls -la\", max_output_tokens: 6000}));\n\
                   text(await tools.write_stdin({session_id:64697,chars:\"\",yield_time_ms:1e3}));\n";
        assert_eq!(
            invocations(src),
            Some(vec![
                (
                    "exec_command".into(),
                    json!({"cmd": "ls -la", "max_output_tokens": 6000})
                ),
                (
                    "write_stdin".into(),
                    json!({"session_id": 64697, "chars": "", "yield_time_ms": 1000.0})
                ),
            ])
        );
        assert_eq!(invocations("  \n"), Some(vec![]));
    }

    #[test]
    fn reads_string_escapes_and_literals() {
        let src = r#"text(await tools.exec_command({'cmd': 'echo \'a\'\nprintf "ç\x41😀"', "x": [true, null, -2, {a: `t`}],}));"#;
        let call = invocations(src).unwrap().remove(0);
        assert_eq!(
            call.1,
            json!({"cmd": "echo 'a'\nprintf \"\u{e7}A\u{1F600}\"", "x": [true, null, -2, {"a": "t"}]})
        );
        let patch =
            invocations("text(await tools.apply_patch(\"*** Begin Patch\\n*** End Patch\"));");
        assert_eq!(
            patch,
            Some(vec![(
                "apply_patch".into(),
                json!("*** Begin Patch\n*** End Patch")
            )])
        );
    }

    #[test]
    fn a_surrogate_pairs_only_with_a_low_one() {
        let arg = |src: &str| {
            invocations(&format!("text(await tools.x({src}));")).unwrap()[0]
                .1
                .clone()
        };
        assert_eq!(arg(r#""a\uD83D\uE000""#), json!("a\u{FFFD}\u{E000}"));
        assert_eq!(arg(r#""a\uD83D\u0041""#), json!("a\u{FFFD}A"));
        assert_eq!(arg(r#""a\uD83Dz""#), json!("a\u{FFFD}z"));
        assert_eq!(arg(r#""\uD83D\uDE00""#), json!("\u{1F600}"));
    }

    #[test]
    fn a_backslash_before_any_line_break_continues_the_string() {
        let src = "text(await tools.x(\"a\\\r\nb\\\nc\\\rd\\\u{2028}e\\\u{2029}f\"));";
        assert_eq!(invocations(src).unwrap()[0].1, json!("abcdef"));
    }

    #[test]
    fn unreadable_programs_are_none() {
        for src in [
            "const r = await tools.exec_command({cmd:\"ls\"}); text(r.output);",
            "text(await tools.exec_command({cmd:`echo ${x}`}));",
            "text(await tools.exec_command({cmd:cmd}));",
            "text(await tools.exec_command({cmd:\"unterminated}));",
            "text(await tools.exec_command({cmd:\"ls\"})",
            "for (const c of cs) text(await tools.exec_command({cmd:c}));",
            "text(await tools.exec_command({cmd:\"a\\u12\"}));",
            "text(await tools.exec_command({cmd:\"a\\",
            "text(await tools.exec_command(",
        ] {
            assert_eq!(invocations(src), None, "{src}");
        }
        let deep = format!(
            "text(await tools.x({}1{}));",
            "[".repeat(40),
            "]".repeat(40)
        );
        assert_eq!(invocations(&deep), None);
    }
}
