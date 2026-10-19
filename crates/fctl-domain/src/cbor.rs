//! A small CBOR decoder (RFC 8949), for what passkeys send: WebAuthn
//! attestation objects and COSE keys. It takes definite lengths only, as
//! CTAP2's canonical CBOR has them, refuses anything nested deeper than it
//! needs, and never allocates more than the input could hold.

/// A decoded CBOR item.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An unsigned integer, or a negative one (`-1 - n` for major type 1).
    Int(i128),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    /// Keys and values in the order they came.
    Map(Vec<(Value, Value)>),
    Bool(bool),
    Null,
    Undefined,
    Float(f64),
    Tag(u64, Box<Value>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ends inside an item.
    Truncated,
    /// Something CTAP2 doesn't send: an indefinite length, a reserved
    /// value, text that isn't UTF-8, or too deep a nesting.
    Unsupported,
}

/// How deep arrays, maps and tags may nest.
const MAX_DEPTH: usize = 16;

impl Value {
    /// The value under `key` in a map.
    pub fn get(&self, key: &Value) -> Option<&Value> {
        match self {
            Self::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The value under an integer key (COSE keys use them).
    pub fn get_int(&self, key: i128) -> Option<&Value> {
        self.get(&Value::Int(key))
    }

    /// The value under a text key (attestation objects use them).
    pub fn get_text(&self, key: &str) -> Option<&Value> {
        self.get(&Value::Text(key.to_owned()))
    }

    pub fn as_int(&self) -> Option<i128> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// Decodes the item at the start of `input`; returns it and how many bytes
/// it took (more may follow).
pub fn decode(input: &[u8]) -> Result<(Value, usize), Error> {
    let mut reader = Reader { input, at: 0 };
    let value = reader.item(0)?;
    Ok((value, reader.at))
}

/// Decodes `input`, which must hold exactly one item.
pub fn decode_all(input: &[u8]) -> Result<Value, Error> {
    let (value, used) = decode(input)?;
    if used == input.len() {
        Ok(value)
    } else {
        Err(Error::Unsupported)
    }
}

struct Reader<'a> {
    input: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, Error> {
        let b = *self.input.get(self.at).ok_or(Error::Truncated)?;
        self.at += 1;
        Ok(b)
    }

    fn take(&mut self, n: u64) -> Result<&[u8], Error> {
        let n = usize::try_from(n).map_err(|_| Error::Truncated)?;
        let end = self.at.checked_add(n).ok_or(Error::Truncated)?;
        let bytes = self.input.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        Ok(bytes)
    }

    /// The argument of a head whose additional information is `info`.
    fn argument(&mut self, info: u8) -> Result<u64, Error> {
        let width = match info {
            0..=23 => return Ok(u64::from(info)),
            24 => 1,
            25 => 2,
            26 => 4,
            27 => 8,
            // 28-30 are reserved, 31 is an indefinite length.
            _ => return Err(Error::Unsupported),
        };
        Ok(self
            .take(width)?
            .iter()
            .fold(0u64, |n, b| (n << 8) | u64::from(*b)))
    }

    /// A count of items that follow, which can't exceed the bytes left.
    fn count(&mut self, info: u8) -> Result<usize, Error> {
        let n = self.argument(info)?;
        let left = (self.input.len() - self.at) as u64;
        if n > left {
            return Err(Error::Truncated);
        }
        usize::try_from(n).map_err(|_| Error::Truncated)
    }

    fn item(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Unsupported);
        }
        let head = self.byte()?;
        let (major, info) = (head >> 5, head & 0x1f);
        Ok(match major {
            0 => Value::Int(i128::from(self.argument(info)?)),
            1 => Value::Int(-1 - i128::from(self.argument(info)?)),
            2 => {
                let n = self.argument(info)?;
                Value::Bytes(self.take(n)?.to_vec())
            }
            3 => {
                let n = self.argument(info)?;
                let text = std::str::from_utf8(self.take(n)?).map_err(|_| Error::Unsupported)?;
                Value::Text(text.to_owned())
            }
            4 => {
                let n = self.count(info)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.item(depth + 1)?);
                }
                Value::Array(items)
            }
            5 => {
                let n = self.count(info)?;
                let mut entries = Vec::with_capacity(n);
                for _ in 0..n {
                    let key = self.item(depth + 1)?;
                    let value = self.item(depth + 1)?;
                    entries.push((key, value));
                }
                Value::Map(entries)
            }
            6 => {
                let tag = self.argument(info)?;
                Value::Tag(tag, Box::new(self.item(depth + 1)?))
            }
            _ => match info {
                20 => Value::Bool(false),
                21 => Value::Bool(true),
                22 => Value::Null,
                23 => Value::Undefined,
                25 => Value::Float(half(self.argument(info)? as u16)),
                26 => Value::Float(f64::from(f32::from_bits(self.argument(info)? as u32))),
                27 => Value::Float(f64::from_bits(self.argument(info)?)),
                _ => return Err(Error::Unsupported),
            },
        })
    }
}

/// An IEEE 754 half-precision float.
fn half(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f64::from(bits & 0x3ff);
    sign * match exponent {
        0 => fraction * 2f64.powi(-24),
        31 if fraction == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + fraction / 1024.0) * 2f64.powi(exponent - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
            .collect()
    }

    #[test]
    fn rfc8949_appendix_a_examples() {
        for (encoded, value) in [
            ("00", Value::Int(0)),
            ("17", Value::Int(23)),
            ("1818", Value::Int(24)),
            ("1903e8", Value::Int(1000)),
            ("1bffffffffffffffff", Value::Int(18_446_744_073_709_551_615)),
            ("20", Value::Int(-1)),
            ("3863", Value::Int(-100)),
            (
                "3bffffffffffffffff",
                Value::Int(-18_446_744_073_709_551_616),
            ),
            ("f4", Value::Bool(false)),
            ("f5", Value::Bool(true)),
            ("f6", Value::Null),
            ("f7", Value::Undefined),
            ("f93c00", Value::Float(1.0)),
            ("f97bff", Value::Float(65504.0)),
            ("fa47c35000", Value::Float(100_000.0)),
            ("fb3ff199999999999a", Value::Float(1.1)),
            ("4401020304", Value::Bytes(vec![1, 2, 3, 4])),
            ("6449455446", Value::Text("IETF".into())),
            ("62c3bc", Value::Text("ü".into())),
            (
                "83010203",
                Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
            ),
            (
                "a26161016162820203",
                Value::Map(vec![
                    (Value::Text("a".into()), Value::Int(1)),
                    (
                        Value::Text("b".into()),
                        Value::Array(vec![Value::Int(2), Value::Int(3)]),
                    ),
                ]),
            ),
            (
                "c11a514b67b0",
                Value::Tag(1, Box::new(Value::Int(1_363_896_240))),
            ),
        ] {
            assert_eq!(decode_all(&hex(encoded)), Ok(value), "{encoded}");
        }
    }

    #[test]
    fn says_how_much_it_read() {
        let (value, used) = decode(&hex("0102")).unwrap_or((Value::Null, 0));
        assert_eq!((value, used), (Value::Int(1), 1));
        assert_eq!(decode_all(&hex("0102")), Err(Error::Unsupported));
    }

    #[test]
    fn refuses_what_ctap2_never_sends() {
        // Truncated items, lengths beyond the input, indefinite lengths,
        // reserved values, bad UTF-8, nesting without end.
        for bad in [
            "",
            "18",
            "4401",
            "5bffffffffffffffff",
            "9bffffffffffffffff00",
        ] {
            assert_eq!(decode(&hex(bad)), Err(Error::Truncated), "{bad}");
        }
        for bad in ["5f", "9f", "bf", "1c", "62c328", "f8"] {
            assert_eq!(decode(&hex(bad)), Err(Error::Unsupported), "{bad}");
        }
        assert_eq!(decode(&[0x81; 64]), Err(Error::Unsupported));
    }

    #[test]
    fn finds_values_by_key() {
        let map = decode_all(&hex("a201020363666f6f")).unwrap_or(Value::Null);
        assert_eq!(map.get_int(1).and_then(Value::as_int), Some(2));
        assert_eq!(map.get_int(3).and_then(Value::as_text), Some("foo"));
        assert_eq!(map.get_int(4), None);
        assert_eq!(Value::Int(1).get_int(1), None);
    }
}
