use agora_vt::{AlacrittyTerminal, Size, Terminal};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn vt(mode: &str) {
    let cols = if mode == "feed-zero" { 0 } else { 1 };
    let mut t = AlacrittyTerminal::new(Size { cols, rows: 1 });
    if mode == "reflow" {
        t.resize(Size { cols: 80, rows: 24 });
        t.feed("日本語 ✅ café e\u{301}".as_bytes());
    }
    for size in [
        Size { cols, rows: 1 },
        Size { cols: 0, rows: 0 },
        Size { cols: 80, rows: 24 },
    ] {
        t.resize(size);
        t.feed("a日✅b".as_bytes());
        assert!(t.size().cols >= 2 && t.size().rows >= 1);
        let c = t.cursor();
        assert!(c.col < t.size().cols && c.row < t.size().rows);
    }
    println!("{mode}: wide text and down/up reflow preserve valid size/cursor");
}

fn node(path: PathBuf) {
    use agora_node::terminal::{Output, TerminalHandle, TerminalOptions};
    use agora_platform::{spawn, PtyConfig};
    let cfg = PtyConfig::new(vec![
        "sh".into(),
        "-c".into(),
        "exec 3>\"$1\"; while IFS= read -r line; do stty size >&3; done".into(),
        "fixture".into(),
        path.to_string_lossy().into_owned(),
    ]);
    let h = TerminalHandle::start(spawn(&cfg).unwrap(), TerminalOptions::default()).unwrap();
    let (_, sub) = h.subscribe();
    let sizes = [
        Size { cols: 0, rows: 0 },
        Size { cols: 1, rows: 1 },
        Size { cols: 0, rows: 1 },
        Size { cols: 80, rows: 24 },
        Size { cols: 1, rows: 1 },
        Size { cols: 80, rows: 24 },
    ];
    let mut expected = Vec::new();
    for (i, size) in sizes.into_iter().enumerate() {
        h.resize(size).unwrap();
        let real = Size {
            cols: size.cols.max(2),
            rows: size.rows.max(1),
        };
        assert_eq!(h.snapshot().size, real);
        h.send_input(b"go\n".to_vec()).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        let said = loop {
            let said = std::fs::read_to_string(&path).unwrap_or_default();
            if said.lines().count() == i + 1 {
                break said;
            }
            assert!(Instant::now() < until, "process never reported size");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            said.lines().last().unwrap(),
            format!("{} {}", real.rows, real.cols)
        );
        if expected.last() != Some(&real) {
            expected.push(real);
        }
    }
    let mut seen = Vec::new();
    while let Ok(out) = sub.recv_timeout(Duration::from_millis(50)) {
        if let Output::Resize(size) = out {
            seen.push(size);
        }
    }
    assert_eq!(seen, expected);
    assert_eq!(seen.len(), 4, "duplicate clamped sizes generate no resize");
    h.kill().unwrap();
    println!(
        "node: 6 requests agree across screen, real PTY stty and broadcasts; 4 effective resizes"
    );
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args[1] == "node" {
        node(PathBuf::from(&args[2]));
    } else {
        vt(&args[1]);
    }
}
