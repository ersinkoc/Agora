//! Real Parser -> App::handle evidence. No terminal, hub or user data is used.
use agora_tui::app::{Action, App, Area, Ctx, Event, Key, Row};
use agora_tui::input::Parser;
use agora_vt::Modes;
use std::time::Instant;

fn parsed(bytes: &[u8]) -> Vec<Event> {
    let mut parser = Parser::default();
    let mut events = parser.push(bytes);
    events.extend(parser.flush());
    events
}

fn linear_reference(text: &[u8]) -> Vec<u8> {
    let mut stack = Vec::with_capacity(text.len());
    for b in text {
        stack.push(*b);
        if stack.ends_with(b"\x1b[200~") || stack.ends_with(b"\x1b[201~") {
            stack.truncate(stack.len() - 6);
        }
    }
    [b"\x1b[200~".as_slice(), &stack, b"\x1b[201~"].concat()
}

fn main() {
    // Controls: all ESC+C0/DEL pairs preserve the independently parsed keys.
    for c in (0..=31).chain(std::iter::once(127)) {
        let mut expected = parsed(b"\x1b");
        expected.extend(parsed(&[c]));
        assert_eq!(parsed(&[27, c]), expected, "control byte {c}");
    }
    for text in ["ç", "ğ", "ş", "😀"] {
        for split in 1..text.len() {
            let mut parser = Parser::default();
            assert!(parser.push(&text.as_bytes()[..split]).is_empty());
            assert_eq!(
                parser.push(&text.as_bytes()[split..]),
                parsed(text.as_bytes())
            );
        }
    }
    let rows = [Row {
        id: "synthetic/agent".into(),
        label: "synthetic".into(),
        state: "working".into(),
        needs_you: false,
        terminal: true,
        header: false,
        group: false,
        request: false,
    }];
    let text = |id: &str, _: &[(&str, agora_i18n::Arg)]| id.to_string();
    let cx = Ctx {
        rows: &rows,
        area: Area::new(0, 0, 120, 30),
        modes: Modes {
            bracketed_paste: true,
            ..Modes::default()
        },
        t: &text,
    };
    let mut app = App::default();
    app.handle(Event::Key(Key::Enter), &cx);
    let mut records = Vec::new();
    for depth in [2_000, 4_000, 8_000, 16_000, 32_000] {
        // There is no end marker in this payload. The nested START markers are
        // therefore accepted as a single paste by the actual inbound parser.
        let nested = format!(
            "{}\x1b[200~{}tail\r",
            "\x1b[200".repeat(depth),
            "~".repeat(depth)
        );
        assert!(!nested.contains("\x1b[201~"));
        let wire = [b"\x1b[200~".as_slice(), nested.as_bytes(), b"\x1b[201~"].concat();
        let mut events = parsed(&wire);
        assert_eq!(events, vec![Event::Paste(nested.clone())]);
        let plain = "x".repeat(nested.len());
        let start = Instant::now();
        let plain_actions = app.handle(Event::Paste(plain.clone()), &cx);
        let plain_ns = start.elapsed().as_nanos();
        assert_eq!(
            plain_actions,
            vec![Action::Input(linear_reference(plain.as_bytes()))]
        );
        let mut times = Vec::new();
        for _ in 0..3 {
            let event = events[0].clone();
            let start = Instant::now();
            let actions = app.handle(event, &cx);
            times.push(start.elapsed().as_nanos());
            assert_eq!(
                actions,
                vec![Action::Input(b"\x1b[200~tail\r\x1b[201~".to_vec())]
            );
        }
        times.sort_unstable();
        let start = Instant::now();
        let reference = linear_reference(nested.as_bytes());
        let linear_ns = start.elapsed().as_nanos();
        assert_eq!(reference, b"\x1b[200~tail\r\x1b[201~");
        records.push(serde_json::json!({"depth": depth, "bytes": nested.len(),
            "actual_app_ns":times,"median_app_ns":times[1],"plain_app_ns":plain_ns,
            "linear_reference_ns":linear_ns,"parser_paste_events":events.len()}));
        events.clear();
    }
    println!(
        "{}",
        serde_json::json!({"head":"dd9a77e8099a84a0c857636dec617a0c9af5c480",
        "scope":"release build, real Parser and App::handle; linear reference is reviewer code",
        "esc_control_cases":33,"utf8_split_controls":"pass","samples":records})
    );
}
