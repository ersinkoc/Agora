// The shell's commands, each with its own permission (`allow-<command>`), so a capability names exactly the ones a page may call:
// the shell's own screen all of them but the page's notification, the hub's page that one only (`capabilities/`, ADR-0048 Amendment A).
const COMMANDS: &[&str] = &[
    "ca_fingerprint",
    "saved",
    "connect",
    "forget",
    "use_locale",
    "notify_agents",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("the desktop shell's build script failed");
}
