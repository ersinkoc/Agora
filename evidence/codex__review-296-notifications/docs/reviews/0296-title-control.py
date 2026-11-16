"""Execute the unchanged title callback body with mock window state, not a GUI."""
from pathlib import Path
import subprocess
import tempfile

source = Path("apps/desktop/src-tauri/src/main.rs").read_text()


def block(marker):
    start = source.index(marker)
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


waiting = block("fn waiting(")
closure = block("move |window, title|")
driver = r'''
use std::sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}};
#[derive(Clone, Default)]
struct App { notices: Arc<Mutex<Vec<u32>>>, titles: Arc<Mutex<Vec<String>>> }
struct Window { app: App, visible: Result<bool, ()>, minimized: Result<bool, ()>, focused: Result<bool, ()> }
impl Window {
    fn set_title(&self, t: &str) -> Result<(), ()> { self.app.titles.lock().unwrap().push(t.into()); Ok(()) }
    fn is_visible(&self) -> Result<bool, ()> { self.visible }
    fn is_minimized(&self) -> Result<bool, ()> { self.minimized }
    fn is_focused(&self) -> Result<bool, ()> { self.focused }
    fn app_handle(&self) -> &App { &self.app }
}
fn notify_waiting(app: &App, count: u32) { app.notices.lock().unwrap().push(count); }
fn typed<F: Fn(Window, String)>(f: F) -> F { f }
fn main() {
    let app = App::default();
    let last = Arc::new(AtomicU32::new(0));
    let callback = typed(CLOSURE);
    let apply = |n: &str, visible, minimized, focused| callback(Window { app: app.clone(), visible, minimized, focused }, n.into());
    apply("(1) product", Ok(true), Ok(false), Ok(true)); // foreground: no notice
    apply("(2) product", Ok(false), Ok(false), Ok(true)); // hidden
    apply("(3) product", Ok(true), Ok(true), Ok(true)); // minimized
    apply("(4) product", Ok(true), Ok(false), Ok(false)); // unfocused
    apply("(4) product", Ok(false), Ok(false), Ok(false)); // same count
    apply("(2) product", Ok(false), Ok(false), Ok(false)); // lower count
    apply("product", Ok(false), Ok(false), Ok(false)); // no prefix
    apply("(x) product", Ok(false), Ok(false), Ok(false)); // invalid prefix
    apply("(4294967296) product", Ok(false), Ok(false), Ok(false)); // overflow
    apply("(5) product", Err(()), Err(()), Err(())); // fallbacks: treated as visible/focused
    assert_eq!(*app.notices.lock().unwrap(), vec![2, 3, 4]);
    assert_eq!(app.titles.lock().unwrap().len(), 10);
    for (tag, one, two) in [
        ("en", "An approval is waiting for you.", "2 approvals are waiting for you."),
        ("tr-TR", "Sizi bekleyen bir onay var.", "Sizi bekleyen 2 onay var."),
    ] {
        let cat = agora_i18n::catalog::Catalog::for_locale(tag);
        for (n, expected) in [(1, one), (2, two)] {
            assert_eq!(cat.render("notify-desktop-approvals", &[("count", agora_i18n::catalog::Arg::Num(n))]).unwrap().trim(), expected);
        }
    }
    println!("10 callback inputs: 3 expected notices; foreground/repeat/decrease/invalid/overflow/error fallbacks pass; title forwarding 10/10; en/tr singular/plural 4/4 pass.");
}
'''.replace("CLOSURE", closure)
libs = list(Path("target/debug/deps").glob("libagora_i18n*.rlib"))
assert len(libs) == 1, libs
with tempfile.TemporaryDirectory(prefix="0296-title-", dir="target") as temporary:
    work = Path(temporary)
    rust = work / "proof.rs"
    exe = work / "proof"
    rust.write_text(waiting + "\n" + driver)
    subprocess.run(["rustc", "--edition=2021", str(rust), "--extern", "agora_i18n=" + str(libs[0]),
                    "-L", "dependency=target/debug/deps", "-o", str(exe)], check=True)
    subprocess.run([str(exe)], check=True)
