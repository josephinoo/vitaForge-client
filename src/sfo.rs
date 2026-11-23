//! Minimal param.sfo reader (PSF): returns string/int entries by key.
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(u32),
}

fn u16le(b: &[u8], o: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?) as usize)
}
fn u32le(b: &[u8], o: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize)
}

pub fn parse(b: &[u8]) -> Option<HashMap<String, Value>> {
    if b.get(..4)? != b"\0PSF" {
        return None;
    }
    let (key_tab, data_tab, n) = (u32le(b, 8)?, u32le(b, 12)?, u32le(b, 16)?);
    let mut out = HashMap::new();
    for i in 0..n {
        let e = 20 + i * 16;
        let key_off = key_tab + u16le(b, e)?;
        let fmt = u16le(b, e + 2)?;
        let len = u32le(b, e + 4)?;
        let data_off = data_tab + u32le(b, e + 12)?;
        let key_end = b.get(key_off..)?.iter().position(|&c| c == 0)? + key_off;
        let key = String::from_utf8_lossy(&b[key_off..key_end]).into_owned();
        let raw = b.get(data_off..data_off + len)?;
        let v = match fmt {
            0x0404 => Value::Int(u32::from_le_bytes(raw.get(..4)?.try_into().ok()?)),
            _ => Value::Str(
                String::from_utf8_lossy(raw.split(|&c| c == 0).next().unwrap_or(&[])).into_owned(),
            ),
        };
        out.insert(key, v);
    }
    Some(out)
}

pub fn get_str(m: &HashMap<String, Value>, k: &str) -> Option<String> {
    match m.get(k)? {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
pub fn build(entries: &[(&str, &str)]) -> Vec<u8> {
    // test helper: all-string SFO
    let mut keys = Vec::new();
    let mut data = Vec::new();
    let mut tab = Vec::new();
    for (k, v) in entries {
        tab.extend((keys.len() as u16).to_le_bytes());
        tab.extend(0x0204u16.to_le_bytes());
        let l = v.len() as u32 + 1;
        tab.extend(l.to_le_bytes());
        tab.extend(l.to_le_bytes());
        tab.extend((data.len() as u32).to_le_bytes());
        keys.extend(k.bytes());
        keys.push(0);
        data.extend(v.bytes());
        data.push(0);
    }
    let key_off = 20 + tab.len() as u32;
    let data_off = key_off + keys.len() as u32;
    let mut o = b"\0PSF\x01\x01\0\0".to_vec();
    o.extend(key_off.to_le_bytes());
    o.extend(data_off.to_le_bytes());
    o.extend((entries.len() as u32).to_le_bytes());
    o.extend(tab);
    o.extend(keys);
    o.extend(data);
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_title() {
        let m = parse(&build(&[("TITLE", "Hello"), ("TITLE_ID", "ABCD12345")])).unwrap();
        assert_eq!(get_str(&m, "TITLE").unwrap(), "Hello");
        assert_eq!(get_str(&m, "TITLE_ID").unwrap(), "ABCD12345");
        assert!(parse(b"nope").is_none());
    }
}
