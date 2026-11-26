//! `agora plugin new | check | pack` (M5.7, the plugin author kit), run as the binary. `pack` makes the package
//! `scripts/pack-plugin.mjs` makes, byte for byte: both are held to the packages in `fixtures/plugin-kit/` (made by the mjs; the web
//! tests hold the mjs to them too, `apps/web/src/plugins/kit.test.ts`), and where `node` is installed the mjs is also run here on the
//! same folders.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugin-kit")
}

/// A fresh folder of this test's own.
fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-plugin-kit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn agora(dir: &Path, args: &[&str]) -> Output {
    // AGORA_LANG comes first of the locale variables (`agora_i18n::catalog::locale_from_env`): the sentences asserted are English.
    Command::new(AGORA)
        .args(args)
        .current_dir(dir)
        .env("AGORA_LANG", "en")
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
}

/// The mjs's package of a folder, when `node` is installed.
fn mjs(dir: &Path, write_hashes: bool) -> Option<Vec<u8>> {
    let mut c = Command::new("node");
    c.arg(root().join("scripts/pack-plugin.mjs")).arg(dir);
    if write_hashes {
        c.arg("--write-hashes");
    }
    let out = c.stderr(Stdio::null()).output().ok()?;
    assert!(out.status.success(), "the mjs packs {}", dir.display());
    assert!(
        !out.stdout.is_empty(),
        "the mjs ran as a script and printed the package"
    );
    Some(out.stdout)
}

#[test]
fn pack_makes_the_mjs_package_byte_for_byte() {
    let dir = temp("golden");
    let order = fixtures().join("listed-order");
    let golden = std::fs::read(fixtures().join("listed-order.bundle")).unwrap();
    // The files in the manifest's own order (not sorted), the manifest's bytes as written, a CRLF script, a PNG, and neither the README
    // nor the types packed.
    let out = agora(&dir, &["plugin", "pack", order.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(out.stdout, golden);
    let file = dir.join("p.json");
    let out = agora(
        &dir,
        &[
            "plugin",
            "pack",
            order.to_str().unwrap(),
            "--out",
            file.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(std::fs::read(&file).unwrap(), golden);
    assert!(
        text(&out.stdout).contains("Packed"),
        "{}",
        text(&out.stdout)
    );

    // --write-hashes rewrites the manifest as JSON.stringify(manifest, null, 2) does, `files` in its own place.
    let copy = dir.join("write-hashes");
    copy_dir(&fixtures().join("write-hashes"), &copy);
    let out = agora(
        &dir,
        &["plugin", "pack", copy.to_str().unwrap(), "--write-hashes"],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        out.stdout,
        std::fs::read(fixtures().join("write-hashes.bundle")).unwrap()
    );
    assert_eq!(
        std::fs::read(copy.join("agora-plugin.json")).unwrap(),
        std::fs::read(fixtures().join("write-hashes.manifest")).unwrap()
    );
    assert!(
        text(&out.stderr).contains("Wrote the files' hashes"),
        "{}",
        text(&out.stderr)
    );

    // Where node is installed, the mjs itself agrees on these folders and on the sample and the starter.
    for folder in [
        order,
        root().join("apps/web/plugins/sample"),
        root().join("apps/web/plugins/starter"),
    ] {
        let Some(theirs) = mjs(&folder, false) else {
            eprintln!("node is not installed: the mjs is held to the same packages by apps/web/src/plugins/kit.test.ts");
            break;
        };
        let ours = agora(&dir, &["plugin", "pack", folder.to_str().unwrap()]);
        assert!(ours.status.success(), "{}", text(&ours.stderr));
        assert_eq!(ours.stdout, theirs, "{}", folder.display());
    }
    let again = dir.join("write-hashes-mjs");
    copy_dir(&fixtures().join("write-hashes"), &again);
    if let Some(theirs) = mjs(&again, true) {
        assert_eq!(
            theirs,
            std::fs::read(fixtures().join("write-hashes.bundle")).unwrap()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_makes_a_plugin_that_packs_and_passes_and_never_overwrites() {
    let dir = temp("new");
    let out = agora(&dir, &["plugin", "new", "hello-world"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let folder = dir.join("hello-world");
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "README.md",
            "agora-plugin.d.ts",
            "agora-plugin.json",
            "panel.css",
            "panel.js"
        ]
    );
    // The scaffold's files are the starter's, the types included, byte for byte.
    let starter = root().join("apps/web/plugins/starter");
    for n in ["README.md", "agora-plugin.d.ts", "panel.css", "panel.js"] {
        assert_eq!(
            std::fs::read(folder.join(n)).unwrap(),
            std::fs::read(starter.join(n)).unwrap(),
            "{n}"
        );
    }
    let manifest = text(&std::fs::read(folder.join("agora-plugin.json")).unwrap());
    assert!(
        manifest.starts_with("{\n  \"id\": \"hello-world\",\n  \"name\": \"hello-world\",\n"),
        "{manifest}"
    );

    // Untouched, it checks, packs (nothing to rewrite) and the package checks.
    let check = agora(&dir, &["plugin", "check", "hello-world"]);
    assert!(check.status.success(), "{}", text(&check.stderr));
    assert!(text(&check.stdout).contains("passes the window's checks"));
    let pack = agora(
        &dir,
        &[
            "plugin",
            "pack",
            "hello-world",
            "--out",
            "hello-world.json",
            "--write-hashes",
        ],
    );
    assert!(pack.status.success(), "{}", text(&pack.stderr));
    assert!(
        !text(&pack.stdout).contains("Wrote the files' hashes"),
        "nothing to rewrite: {}",
        text(&pack.stdout)
    );
    let check = agora(&dir, &["plugin", "check", "hello-world.json"]);
    assert!(check.status.success(), "{}", text(&check.stderr));
    if let Some(theirs) = mjs(&folder, false) {
        assert_eq!(std::fs::read(dir.join("hello-world.json")).unwrap(), theirs);
    }

    // It never writes into what exists, and refuses an id the window would.
    std::fs::write(folder.join("panel.js"), "mine").unwrap();
    let again = agora(&dir, &["plugin", "new", "hello-world"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(
        text(&again.stderr).contains("already exists"),
        "{}",
        text(&again.stderr)
    );
    assert_eq!(std::fs::read(folder.join("panel.js")).unwrap(), b"mine");
    std::fs::write(dir.join("taken"), "x").unwrap();
    assert_eq!(
        agora(&dir, &["plugin", "new", "x", "taken"]).status.code(),
        Some(1)
    );
    for bad in ["Hello", "a/b", "../up", "-x", "a_b"] {
        let out = agora(&dir, &["plugin", "new", bad]);
        assert_eq!(out.status.code(), Some(1), "{bad}");
        assert!(
            text(&out.stderr).contains("cannot be a plugin's id"),
            "{bad}: {}",
            text(&out.stderr)
        );
    }
    assert!(!dir.join("up").exists() && !dir.join("a").exists());
    let other = agora(&dir, &["plugin", "new", "x", "elsewhere"]);
    assert!(other.status.success());
    assert!(dir.join("elsewhere/agora-plugin.json").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder from the starter, ready to break.
fn plugin_folder(dir: &Path, id: &str) -> PathBuf {
    assert!(agora(dir, &["plugin", "new", id]).status.success());
    dir.join(id)
}

fn refused(out: &Output, code: &str) {
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains(&format!("({code})")),
        "wanted {code}: {}",
        text(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "nothing packed");
}

#[test]
fn check_and_pack_refuse_what_the_window_would() {
    let dir = temp("refuse");
    // A changed file without its hash.
    let f = plugin_folder(&dir, "changed");
    std::fs::write(f.join("panel.js"), "agora.read('agents.read');").unwrap();
    refused(&agora(&dir, &["plugin", "check", "changed"]), "hash");
    refused(&agora(&dir, &["plugin", "pack", "changed"]), "hash");
    // A file the manifest does not list, and a sub-folder.
    let f = plugin_folder(&dir, "extra");
    std::fs::write(f.join("more.js"), "1").unwrap();
    refused(&agora(&dir, &["plugin", "pack", "extra"]), "unlisted");
    let f = plugin_folder(&dir, "nested");
    std::fs::create_dir(f.join("lib")).unwrap();
    refused(
        &agora(&dir, &["plugin", "pack", "nested", "--write-hashes"]),
        "not-a-file",
    );
    // --write-hashes lists the file, and then the window's own checks refuse its kind.
    let f = plugin_folder(&dir, "kind");
    std::fs::write(f.join("tool.exe"), "MZ").unwrap();
    refused(
        &agora(&dir, &["plugin", "pack", "kind", "--write-hashes"]),
        "type",
    );
    // A manifest naming a file outside the folder: the name is never read, the pack is refused.
    let outside = dir.join("outside.js");
    std::fs::write(&outside, "secret").unwrap();
    let f = plugin_folder(&dir, "escape");
    let m = std::fs::read_to_string(f.join("agora-plugin.json")).unwrap();
    let hash = "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b";
    std::fs::write(
        f.join("agora-plugin.json"),
        m.replace(
            "\"files\": {",
            &format!("\"files\": {{\n    \"../outside.js\": \"{hash}\","),
        ),
    )
    .unwrap();
    refused(&agora(&dir, &["plugin", "pack", "escape"]), "missing");
    // A capability the window does not know.
    let f = plugin_folder(&dir, "asks");
    let m = std::fs::read_to_string(f.join("agora-plugin.json")).unwrap();
    std::fs::write(
        f.join("agora-plugin.json"),
        m.replace("\"theme.read\"", "\"terminal.read\""),
    )
    .unwrap();
    refused(&agora(&dir, &["plugin", "check", "asks"]), "capability");
    // Over 2 MiB, read no further than that.
    let f = plugin_folder(&dir, "big");
    std::fs::write(f.join("big.txt"), vec![b'a'; 2 * 1024 * 1024 + 1]).unwrap();
    refused(
        &agora(&dir, &["plugin", "pack", "big", "--write-hashes"]),
        "too-large",
    );
    std::fs::write(dir.join("big.json"), vec![b' '; 2 * 1024 * 1024 + 1]).unwrap();
    refused(&agora(&dir, &["plugin", "check", "big.json"]), "too-large");
    // A package file of the shared set the hub and the window agree on.
    let hub =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../agora-hub/tests/fixtures/plugin-packages");
    refused(
        &agora(
            &dir,
            &["plugin", "check", hub.join("hash.bundle").to_str().unwrap()],
        ),
        "hash",
    );
    refused(
        &agora(
            &dir,
            &["plugin", "check", hub.join("path.bundle").to_str().unwrap()],
        ),
        "path",
    );
    assert!(agora(
        &dir,
        &["plugin", "check", hub.join("ok.bundle").to_str().unwrap()]
    )
    .status
    .success());
    // Nothing there.
    refused(
        &agora(&dir, &["plugin", "check", "nothing-here"]),
        "unreadable",
    );
    // The package is never written into the folder itself, and --watch needs a file to write.
    plugin_folder(&dir, "here");
    let out = agora(&dir, &["plugin", "pack", "here", "--out", "here/p.json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!dir.join("here/p.json").exists());
    assert_eq!(
        agora(&dir, &["plugin", "pack", "here", "--watch"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(agora(&dir, &["plugin", "check"]).status.code(), Some(2));
    assert_eq!(
        agora(&dir, &["plugin", "new", "a", "--out", "x"])
            .status
            .code(),
        Some(2)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_link_in_the_folder_is_refused_and_not_followed() {
    let dir = temp("link");
    let secret = dir.join("secret.txt");
    std::fs::write(&secret, "do not pack").unwrap();
    let f = plugin_folder(&dir, "linked");
    std::os::unix::fs::symlink(&secret, f.join("notes.txt")).unwrap();
    refused(
        &agora(&dir, &["plugin", "pack", "linked", "--write-hashes"]),
        "not-a-file",
    );
    assert!(
        !text(&std::fs::read(f.join("agora-plugin.json")).unwrap()).contains("notes.txt"),
        "nothing was written for it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hidden_files_and_the_readme_and_types_are_not_packed() {
    let dir = temp("hidden");
    let f = plugin_folder(&dir, "tidy");
    let before = agora(&dir, &["plugin", "pack", "tidy"]);
    assert!(before.status.success());
    std::fs::write(f.join(".DS_Store"), "x").unwrap();
    std::fs::write(f.join("README.md"), "changed").unwrap();
    std::fs::write(f.join("agora-plugin.d.ts"), "changed").unwrap();
    let after = agora(&dir, &["plugin", "pack", "tidy"]);
    assert!(after.status.success(), "{}", text(&after.stderr));
    assert_eq!(before.stdout, after.stdout);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Waits until `f` holds something other than `not`, at most 20 s.
fn changed(f: &Path, not: Option<&[u8]>) -> Vec<u8> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(b) = std::fs::read(f) {
            if Some(b.as_slice()) != not {
                return b;
            }
        }
        assert!(Instant::now() < end, "{} did not change", f.display());
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn watch_packs_again_on_every_save_and_keeps_going_past_a_refusal() {
    let dir = temp("watch");
    let f = plugin_folder(&dir, "live");
    let out = dir.join("live.json");
    let mut child = Command::new(AGORA)
        .args([
            "plugin",
            "pack",
            "live",
            "--out",
            "live.json",
            "--write-hashes",
            "--watch",
        ])
        .current_dir(&dir)
        .env("AGORA_LANG", "en")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let first = changed(&out, None);
    std::fs::write(f.join("panel.js"), "agora.read('agents.read');\n").unwrap();
    let second = changed(&out, Some(&first));
    assert!(text(&second).contains(&base64_of("agora.read('agents.read');\n")));
    // A refused folder writes nothing; the next good save is packed.
    std::fs::write(f.join("bad.exe"), "MZ").unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(std::fs::read(&out).unwrap(), second);
    std::fs::remove_file(f.join("bad.exe")).unwrap();
    std::fs::write(f.join("panel.js"), "agora.read('theme.read');\n").unwrap();
    let third = changed(&out, Some(&second));
    assert!(text(&third).contains(&base64_of("agora.read('theme.read');\n")));
    child.kill().unwrap();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

fn base64_of(s: &str) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in s.as_bytes().chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() {
                char::from(ABC[((n >> (18 - 6 * i)) & 63) as usize])
            } else {
                '='
            });
        }
    }
    out
}
