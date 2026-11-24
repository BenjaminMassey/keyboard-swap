//! Parsing VIA layout exports, and encoding their keycode names and macros into the bytes
//! the keyboard stores.

use crate::config::Entry;
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::path::PathBuf;

/// The parts of a VIA "Save + Load" JSON export that we use.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViaExport {
    vendor_product_id: u32,
    layers: Vec<Vec<String>>,
    #[serde(default)]
    macros: Vec<String>,
    #[serde(default)]
    encoders: Vec<serde_json::Value>,
}

pub struct Layout {
    pub name: String,
    pub path: PathBuf,
    pub vid: u16,
    pub pid: u16,
    pub layers: Vec<Vec<String>>,
    pub macros: Vec<String>,
    pub has_encoders: bool,
}

impl Layout {
    pub fn load(entry: &Entry) -> Result<Self> {
        let text = std::fs::read_to_string(&entry.path)
            .with_context(|| format!("could not read layout {}", entry.path.display()))?;
        let export: ViaExport = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a VIA layout export", entry.path.display()))?;

        let keys = export.layers.first().map_or(0, Vec::len);
        if keys == 0 {
            bail!("{}: no layers in layout", entry.path.display());
        }
        if export.layers.iter().any(|l| l.len() != keys) {
            bail!("{}: layers have differing numbers of keys", entry.path.display());
        }
        Ok(Self {
            name: entry.name.clone(),
            path: entry.path.clone(),
            vid: (export.vendor_product_id >> 16) as u16,
            pid: export.vendor_product_id as u16,
            layers: export.layers,
            macros: export.macros,
            has_encoders: !export.encoders.is_empty(),
        })
    }

    pub fn keys_per_layer(&self) -> usize {
        self.layers[0].len()
    }

    pub fn encode_keymap(&self, kc_version: u32) -> Result<Vec<Vec<u16>>> {
        self.layers
            .iter()
            .enumerate()
            .map(|(li, layer)| {
                layer
                    .iter()
                    .enumerate()
                    .map(|(ki, name)| {
                        keycode(name, kc_version).with_context(|| {
                            format!("{}: layer {li}, key {ki}", self.path.display())
                        })
                    })
                    .collect()
            })
            .collect()
    }

    /// Encodes each macro (without its 0x00 terminator), up to the keyboard's macro count.
    pub fn encode_macros(&self, kc_version: u32, count: usize) -> Result<Vec<Vec<u8>>> {
        if self.macros.len() > count && self.macros[count..].iter().any(|m| !m.is_empty()) {
            bail!(
                "{}: has {} macros but the keyboard only holds {count}",
                self.path.display(),
                self.macros.len()
            );
        }
        self.macros
            .iter()
            .take(count)
            .enumerate()
            .map(|(i, m)| {
                encode_macro(m, kc_version)
                    .with_context(|| format!("{}: macro {i} ({m:?})", self.path.display()))
            })
            .collect()
    }
}

/// Converts a VIA keycode name such as `KC_A`, `LT(3,KC_Z)` or `CUSTOM(2)` to its numeric value.
///
/// `kc_version` selects the keycode numbering: 6 for VIA protocol 12+ (QMK 0.19+), 5 for older.
pub fn keycode(name: &str, kc_version: u32) -> Result<u16> {
    let name = name.trim();
    if let Some(hex) = name.strip_prefix("0x").or_else(|| name.strip_prefix("0X")) {
        return u16::from_str_radix(hex, 16).with_context(|| format!("bad hex keycode {name:?}"));
    }
    // VIA writes keyboard-specific keys as CUSTOM(n) and macro keys as MACRO(n); vitaly knows
    // them as USERnn and QK_MACRO_n.
    let numbered = |prefix: &str| -> Result<Option<u8>> {
        match name.strip_prefix(prefix).and_then(|s| s.strip_suffix(')')) {
            Some(n) => Ok(Some(n.trim().parse().with_context(|| format!("bad keycode {name:?}"))?)),
            None => Ok(None),
        }
    };
    let translated = if let Some(n) = numbered("CUSTOM(")? {
        Some(format!("USER{n:02}"))
    } else {
        numbered("MACRO(")?.map(|n| format!("QK_MACRO_{n}"))
    };
    let lookup = translated.as_deref().unwrap_or(name);
    vitaly::keycodes::name_to_qid(lookup, kc_version)
        .or_else(|e| {
            let aliased = replace_aliases(lookup);
            if aliased == lookup {
                Err(e)
            } else {
                vitaly::keycodes::name_to_qid(&aliased, kc_version)
            }
        })
        .map_err(|e| anyhow!("unknown keycode {name:?} ({e})"))
}

/// Older keycode spellings that VIA still exports but vitaly doesn't know.
fn alias(token: &str) -> Option<&'static str> {
    Some(match token {
        "RESET" => "QK_BOOT",
        "KC_MS_UP" => "QK_MOUSE_CURSOR_UP",
        "KC_MS_DOWN" => "QK_MOUSE_CURSOR_DOWN",
        "KC_MS_LEFT" => "QK_MOUSE_CURSOR_LEFT",
        "KC_MS_RIGHT" => "QK_MOUSE_CURSOR_RIGHT",
        "KC_MS_BTN1" => "QK_MOUSE_BUTTON_1",
        "KC_MS_BTN2" => "QK_MOUSE_BUTTON_2",
        "KC_MS_BTN3" => "QK_MOUSE_BUTTON_3",
        "KC_MS_BTN4" => "QK_MOUSE_BUTTON_4",
        "KC_MS_BTN5" => "QK_MOUSE_BUTTON_5",
        "KC_MS_BTN6" => "QK_MOUSE_BUTTON_6",
        "KC_MS_BTN7" => "QK_MOUSE_BUTTON_7",
        "KC_MS_BTN8" => "QK_MOUSE_BUTTON_8",
        "KC_MS_WH_UP" => "QK_MOUSE_WHEEL_UP",
        "KC_MS_WH_DOWN" => "QK_MOUSE_WHEEL_DOWN",
        "KC_MS_WH_LEFT" => "QK_MOUSE_WHEEL_LEFT",
        "KC_MS_WH_RIGHT" => "QK_MOUSE_WHEEL_RIGHT",
        "KC_MS_ACCEL0" => "QK_MOUSE_ACCELERATION_0",
        "KC_MS_ACCEL1" => "QK_MOUSE_ACCELERATION_1",
        "KC_MS_ACCEL2" => "QK_MOUSE_ACCELERATION_2",
        _ => return None,
    })
}

/// Applies [`alias`] to every identifier in a keycode expression, e.g. inside `LT(1,KC_MS_BTN1)`.
fn replace_aliases(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        out.push_str(alias(token).unwrap_or(token));
        token.clear();
    };
    for c in expr.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

// QMK's send_string encoding, as used by VIA's dynamic macros.
const SS_PREFIX: u8 = 0x01;
const SS_TAP: u8 = 0x01;
const SS_DOWN: u8 = 0x02;
const SS_UP: u8 = 0x03;
const SS_DELAY: u8 = 0x04;

fn basic_keycode(name: &str, kc_version: u32) -> Result<u8> {
    let code = keycode(name, kc_version)?;
    u8::try_from(code)
        .map_err(|_| anyhow!("{name:?} is not a basic keycode; only basic keys are supported in macros"))
}

/// Encodes VIA's macro text format: literal characters, `{KC_A}` taps, `{KC_LCTL,KC_C}` chords,
/// `{+KC_A}` / `{-KC_A}` press / release, and `{100}` delays in milliseconds.
pub fn encode_macro(text: &str, kc_version: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '{' {
            if !c.is_ascii() || (c.is_ascii_control() && c != '\n' && c != '\t') {
                bail!("character {c:?} can't be sent by a macro");
            }
            out.push(c as u8);
            continue;
        }
        let inner: String = chars.by_ref().take_while(|&c| c != '}').collect();
        let inner = inner.trim();
        if !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit()) {
            out.extend([SS_PREFIX, SS_DELAY]);
            out.extend(inner.bytes());
            out.push(b'|');
        } else if let Some(key) = inner.strip_prefix('+') {
            out.extend([SS_PREFIX, SS_DOWN, basic_keycode(key, kc_version)?]);
        } else if let Some(key) = inner.strip_prefix('-') {
            out.extend([SS_PREFIX, SS_UP, basic_keycode(key, kc_version)?]);
        } else {
            let keys = inner
                .split(',')
                .map(|k| basic_keycode(k, kc_version))
                .collect::<Result<Vec<_>>>()?;
            if let [key] = keys[..] {
                out.extend([SS_PREFIX, SS_TAP, key]);
            } else {
                for &k in &keys {
                    out.extend([SS_PREFIX, SS_DOWN, k]);
                }
                for &k in keys.iter().rev() {
                    out.extend([SS_PREFIX, SS_UP, k]);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_keycodes() {
        for (name, code) in [
            ("KC_A", 0x0004),
            ("KC_NO", 0x0000),
            ("KC_TRNS", 0x0001),
            ("KC_MS_BTN1", 0x00D1),
            ("S(KC_GRV)", 0x0235),
            ("LT(3,KC_Z)", 0x431D),
            ("MO(1)", 0x5221),
            ("RGB_MOD", 0x7821),
            ("CUSTOM(0)", 0x7E00),
            ("CUSTOM(4)", 0x7E04),
            ("0x7e05", 0x7E05),
            ("KC_MS_WH_UP", 0x00D9),
            ("LT(1,KC_MS_BTN2)", 0x41D2),
            ("MACRO(0)", 0x7700),
            ("MACRO(15)", 0x770F),
            ("RESET", 0x7C00),
        ] {
            assert_eq!(keycode(name, 6).unwrap(), code, "{name}");
        }
    }

    #[test]
    fn legacy_keycodes() {
        assert_eq!(keycode("KC_A", 5).unwrap(), 0x0004);
        assert_eq!(keycode("CUSTOM(0)", 5).unwrap(), 0x5F80);
        assert_eq!(keycode("MACRO(0)", 5).unwrap(), 0x5F12);
        assert_eq!(keycode("RESET", 5).unwrap(), 0x5C00);
    }

    #[test]
    fn unknown_keycode_is_an_error() {
        assert!(keycode("KC_BOGUS", 6).is_err());
    }

    #[test]
    fn macros() {
        assert_eq!(encode_macro("", 6).unwrap(), b"");
        assert_eq!(encode_macro("=", 6).unwrap(), b"=");
        assert_eq!(encode_macro("hi{KC_ENT}", 6).unwrap(), [b'h', b'i', 1, 1, 0x28]);
        assert_eq!(
            encode_macro("{KC_LCTL,KC_C}", 6).unwrap(),
            [1, 2, 0xE0, 1, 2, 0x06, 1, 3, 0x06, 1, 3, 0xE0]
        );
        assert_eq!(encode_macro("{50}", 6).unwrap(), [1, 4, b'5', b'0', b'|']);
        assert!(encode_macro("{MO(1)}", 6).is_err());
    }
}
