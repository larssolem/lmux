use gtk4::gdk;

/// Classification of a key press: app-level scroll actions take precedence over
/// raw PTY input so PageUp/PageDown drive scrollback rather than reaching the
/// shell.
#[derive(Debug, Clone)]
pub enum KeyAction {
    /// Scroll the viewport by `rows`. Negative = up, positive = down.
    ScrollRows(i32),
    /// Write bytes to the PTY.
    Write(Vec<u8>),
}

/// Classify a GDK key press. Returns the action to execute on the focused pane.
pub fn classify_key(keyval: gdk::Key, modifier: gdk::ModifierType, page_rows: u16) -> KeyAction {
    use gdk::Key;
    let shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);
    match keyval {
        Key::Page_Up if shift => KeyAction::ScrollRows(-1),
        Key::Page_Down if shift => KeyAction::ScrollRows(1),
        Key::Page_Up => KeyAction::ScrollRows(-(page_rows.max(1) as i32)),
        Key::Page_Down => KeyAction::ScrollRows(page_rows.max(1) as i32),
        _ => KeyAction::Write(translate_key(keyval, modifier)),
    }
}

/// Translate a GDK key press to the byte sequence the PTY expects.
/// Returns an empty vec for keys we intentionally don't forward.
pub fn translate_key(keyval: gdk::Key, modifier: gdk::ModifierType) -> Vec<u8> {
    if modifier.contains(gdk::ModifierType::CONTROL_MASK) {
        if let Some(ch) = keyval.to_unicode() {
            if ch.is_ascii_alphabetic() {
                let c = ch.to_ascii_lowercase() as u8 - b'a' + 1;
                return vec![c];
            }
        }
    }
    use gdk::Key;
    match keyval {
        Key::Return => vec![b'\r'],
        Key::BackSpace => vec![0x7f],
        Key::Tab => vec![b'\t'],
        Key::Escape => vec![0x1b],
        Key::Up => b"\x1b[A".to_vec(),
        Key::Down => b"\x1b[B".to_vec(),
        Key::Right => b"\x1b[C".to_vec(),
        Key::Left => b"\x1b[D".to_vec(),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        _ => keyval.to_unicode().map(encode_utf8).unwrap_or_default(),
    }
}

fn encode_utf8(ch: char) -> Vec<u8> {
    let mut buf = [0u8; 4];
    ch.encode_utf8(&mut buf).as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_mods() -> gdk::ModifierType {
        gdk::ModifierType::empty()
    }

    #[test]
    fn page_up_scrolls_a_full_page() {
        let out = classify_key(gdk::Key::Page_Up, no_mods(), 24);
        match out {
            KeyAction::ScrollRows(n) => assert_eq!(n, -24),
            other => panic!("expected ScrollRows(-24), got {other:?}"),
        }
    }

    #[test]
    fn shift_page_up_scrolls_one_row() {
        let out = classify_key(gdk::Key::Page_Up, gdk::ModifierType::SHIFT_MASK, 24);
        match out {
            KeyAction::ScrollRows(n) => assert_eq!(n, -1),
            other => panic!("expected ScrollRows(-1), got {other:?}"),
        }
    }

    #[test]
    fn page_down_clamps_at_one_row_minimum() {
        // Even if the page reports 0 rows, scroll by at least 1.
        let out = classify_key(gdk::Key::Page_Down, no_mods(), 0);
        match out {
            KeyAction::ScrollRows(n) => assert_eq!(n, 1),
            other => panic!("expected ScrollRows(1), got {other:?}"),
        }
    }

    #[test]
    fn ctrl_c_maps_to_0x03() {
        let bytes = translate_key(gdk::Key::c, gdk::ModifierType::CONTROL_MASK);
        assert_eq!(bytes, vec![0x03]);
    }

    #[test]
    fn ctrl_letter_case_insensitive() {
        let lower = translate_key(gdk::Key::a, gdk::ModifierType::CONTROL_MASK);
        let upper = translate_key(
            gdk::Key::A,
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
        );
        assert_eq!(lower, vec![0x01]);
        assert_eq!(upper, vec![0x01]);
    }

    #[test]
    fn arrow_keys_emit_csi_sequences() {
        assert_eq!(translate_key(gdk::Key::Up, no_mods()), b"\x1b[A".to_vec());
        assert_eq!(translate_key(gdk::Key::Down, no_mods()), b"\x1b[B".to_vec());
        assert_eq!(
            translate_key(gdk::Key::Right, no_mods()),
            b"\x1b[C".to_vec()
        );
        assert_eq!(translate_key(gdk::Key::Left, no_mods()), b"\x1b[D".to_vec());
    }

    #[test]
    fn enter_and_backspace_use_classic_codes() {
        assert_eq!(translate_key(gdk::Key::Return, no_mods()), vec![b'\r']);
        assert_eq!(translate_key(gdk::Key::BackSpace, no_mods()), vec![0x7f]);
    }

    #[test]
    fn printable_ascii_passes_through() {
        let out = translate_key(gdk::Key::a, no_mods());
        assert_eq!(out, vec![b'a']);
    }

    #[test]
    fn unmapped_key_produces_empty_bytes() {
        // F24 is not in our translate table and doesn't map to a unicode
        // char in GDK — should return an empty Vec (Write action that
        // writes nothing, filtered out by the caller).
        let out = translate_key(gdk::Key::F24, no_mods());
        assert!(out.is_empty(), "expected empty, got {out:?}");
    }
}
