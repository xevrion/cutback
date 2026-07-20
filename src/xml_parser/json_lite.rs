//! Just enough JSON to read Kdenlive's marker and guide properties.
//!
//! Kdenlive stores markers and guides as a JSON array inside an XML property.
//! Both are flat arrays of objects with scalar fields, so a small scanner beats
//! taking a serde dependency for two properties. It is not a general JSON
//! parser and does not try to be: nested arrays and objects inside an element
//! are rejected rather than silently mis-scanned.

/// One `{...}` object, as key/value pairs with quotes and escapes resolved.
pub struct Object {
    fields: Vec<(String, Value)>,
}

pub enum Value {
    Str(String),
    Num(f64),
    Other,
}

impl Object {
    pub fn str(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        })
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        self.fields.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
            Value::Num(n) => Some(*n),
            _ => None,
        })
    }
}

/// Parses a flat array of objects. Returns None if the text is not shaped the
/// way Kdenlive writes these properties, so the caller can fail loudly rather
/// than report an empty marker list for a project that has markers.
pub fn parse_object_array(text: &str) -> Option<Vec<Object>> {
    let mut chars = text.char_indices().peekable();

    skip_ws(&mut chars);
    match chars.next() {
        Some((_, '[')) => {}
        _ => return None,
    }

    let mut objects = Vec::new();
    loop {
        skip_ws(&mut chars);
        match chars.peek() {
            Some((_, ']')) => {
                chars.next();
                return Some(objects);
            }
            Some((_, ',')) => {
                chars.next();
            }
            Some((_, '{')) => {
                chars.next();
                objects.push(parse_object(&mut chars)?);
            }
            _ => return None,
        }
    }
}

type Chars<'a> = std::iter::Peekable<std::str::CharIndices<'a>>;

fn parse_object(chars: &mut Chars<'_>) -> Option<Object> {
    let mut fields = Vec::new();
    loop {
        skip_ws(chars);
        match chars.next() {
            Some((_, '}')) => return Some(Object { fields }),
            Some((_, ',')) => continue,
            Some((_, '"')) => {
                let key = parse_string(chars)?;
                skip_ws(chars);
                if !matches!(chars.next(), Some((_, ':'))) {
                    return None;
                }
                skip_ws(chars);
                fields.push((key, parse_value(chars)?));
            }
            _ => return None,
        }
    }
}

fn parse_value(chars: &mut Chars<'_>) -> Option<Value> {
    match chars.peek() {
        Some((_, '"')) => {
            chars.next();
            Some(Value::Str(parse_string(chars)?))
        }
        Some((_, c)) if c.is_ascii_digit() || *c == '-' => {
            let mut raw = String::new();
            while let Some((_, c)) = chars.peek() {
                if c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E') {
                    raw.push(*c);
                    chars.next();
                } else {
                    break;
                }
            }
            raw.parse().ok().map(Value::Num)
        }
        // Kdenlive does not currently nest anything inside these objects.
        // Skipping a bare literal keeps true/false/null from derailing the scan.
        Some(_) => {
            while let Some((_, c)) = chars.peek() {
                if c.is_alphabetic() {
                    chars.next();
                } else {
                    break;
                }
            }
            Some(Value::Other)
        }
        None => None,
    }
}

fn parse_string(chars: &mut Chars<'_>) -> Option<String> {
    let mut out = String::new();
    loop {
        match chars.next()? {
            (_, '"') => return Some(out),
            (_, '\\') => {
                let (_, esc) = chars.next()?;
                out.push(match esc {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    'u' => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            let (_, d) = chars.next()?;
                            code = code * 16 + d.to_digit(16)?;
                        }
                        char::from_u32(code)?
                    }
                    other => other,
                });
            }
            (_, c) => out.push(c),
        }
    }
}

fn skip_ws(chars: &mut Chars<'_>) {
    while let Some((_, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_real_marker_array() {
        // Copied verbatim from a project file written by Kdenlive 25.x.
        let raw = r#"[
    {
        "comment": "Gap",
        "duration": 0,
        "pos": 2574,
        "type": 2
    }
]"#;
        let objs = parse_object_array(raw).expect("should parse");
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].str("comment"), Some("Gap"));
        assert_eq!(objs[0].num("pos"), Some(2574.0));
        assert_eq!(objs[0].num("type"), Some(2.0));
    }

    #[test]
    fn reads_an_empty_array() {
        assert_eq!(parse_object_array("[\n]").map(|v| v.len()), Some(0));
    }

    #[test]
    fn handles_escapes_and_unicode() {
        let objs = parse_object_array(r#"[{"comment":"a \"quoted\" A line"}]"#).unwrap();
        assert_eq!(objs[0].str("comment"), Some(r#"a "quoted" A line"#));
    }

    #[test]
    fn rejects_non_arrays() {
        assert!(parse_object_array("not json").is_none());
        assert!(parse_object_array("{\"pos\":1}").is_none());
    }
}
