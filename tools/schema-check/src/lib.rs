//! Breaking-change check for `schema/` (ADR-0006, ADR-0009): a published message is never changed.
//!
//! Compares a base schema (for example the target branch) with the current one. Anything the base declared
//! must still exist with the same meaning. Adding messages, fields, enums, and enum values is allowed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use prost_types::{
    field_descriptor_proto::Label, DescriptorProto, EnumDescriptorProto, FieldDescriptorProto,
    FileDescriptorSet,
};

/// Compile every `*.proto` below `schema_root/agora/` (recursively) into one descriptor set.
pub fn load(schema_root: &Path) -> Result<FileDescriptorSet, String> {
    let mut protos = Vec::new();
    collect(&schema_root.join("agora"), &mut protos).map_err(|e| e.to_string())?;
    protos.sort();
    if protos.is_empty() {
        return Ok(FileDescriptorSet::default());
    }
    protox::compile(&protos, [schema_root])
        .map(|fds| FileDescriptorSet { file: fds.file })
        .map_err(|e| e.to_string())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            collect(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "proto") {
            out.push(p);
        }
    }
    Ok(())
}

#[derive(Default)]
struct Index<'a> {
    messages: BTreeMap<String, &'a DescriptorProto>,
    enums: BTreeMap<String, &'a EnumDescriptorProto>,
}

fn index(fds: &FileDescriptorSet) -> Index<'_> {
    fn walk<'a>(prefix: &str, m: &'a DescriptorProto, ix: &mut Index<'a>) {
        let name = format!("{prefix}.{}", m.name());
        for n in &m.nested_type {
            walk(&name, n, ix);
        }
        for e in &m.enum_type {
            ix.enums.insert(format!("{name}.{}", e.name()), e);
        }
        ix.messages.insert(name, m);
    }
    let mut ix = Index::default();
    for f in &fds.file {
        let pkg = f.package();
        for m in &f.message_type {
            walk(pkg, m, &mut ix);
        }
        for e in &f.enum_type {
            ix.enums.insert(format!("{pkg}.{}", e.name()), e);
        }
    }
    ix
}

fn describe(f: &FieldDescriptorProto) -> String {
    let label = match f.label() {
        Label::Repeated => "repeated ",
        _ if f.proto3_optional() => "optional ",
        _ => "",
    };
    format!("{label}{:?} {}", f.r#type(), f.type_name())
}

/// Returns one line per breaking change; empty means compatible.
pub fn breaking_changes(base: &FileDescriptorSet, head: &FileDescriptorSet) -> Vec<String> {
    let (b, h) = (index(base), index(head));
    let mut out = Vec::new();
    for (name, bm) in &b.messages {
        let Some(hm) = h.messages.get(name) else {
            out.push(format!("message {name} was removed"));
            continue;
        };
        for bf in &bm.field {
            match hm.field.iter().find(|f| f.number() == bf.number()) {
                None => out.push(format!(
                    "{name}: field {} = {} was removed",
                    bf.name(),
                    bf.number()
                )),
                Some(hf) => {
                    if hf.name() != bf.name() {
                        out.push(format!(
                            "{name}: field {} renamed to {}",
                            bf.name(),
                            hf.name()
                        ));
                    }
                    if describe(hf) != describe(bf) {
                        out.push(format!(
                            "{name}.{}: type changed from `{}` to `{}`",
                            bf.name(),
                            describe(bf).trim(),
                            describe(hf).trim()
                        ));
                    }
                }
            }
        }
    }
    for (name, be) in &b.enums {
        let Some(he) = h.enums.get(name) else {
            out.push(format!("enum {name} was removed"));
            continue;
        };
        for bv in &be.value {
            match he.value.iter().find(|v| v.number() == bv.number()) {
                None => out.push(format!(
                    "{name}: value {} = {} was removed",
                    bv.name(),
                    bv.number()
                )),
                Some(hv) if hv.name() != bv.name() => {
                    out.push(format!(
                        "{name}: value {} renamed to {}",
                        bv.name(),
                        hv.name()
                    ));
                }
                Some(_) => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fds(tag: &str, proto: &str) -> FileDescriptorSet {
        // Unique per call: tests run in parallel and must not share a directory.
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("agora-schema-{tag}-{}-{n}", std::process::id()));
        let dir = root.join("agora/v1");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&dir).unwrap_or_default();
        std::fs::write(dir.join("t.proto"), proto).unwrap_or_default();
        load(&root).unwrap_or_default()
    }

    const BASE: &str = r#"syntax = "proto3"; package agora.v1;
        enum Color { COLOR_UNSPECIFIED = 0; COLOR_RED = 1; }
        message Thing { string name = 1; repeated uint32 ids = 2; Color color = 3; }"#;

    fn check(head: &str) -> Vec<String> {
        let base = fds("base", BASE);
        assert!(!base.file.is_empty(), "base schema failed to compile");
        let head = fds("head", head);
        assert!(!head.file.is_empty(), "head schema failed to compile");
        breaking_changes(&base, &head)
    }

    #[test]
    fn unchanged_and_additive_changes_are_fine() {
        assert!(check(BASE).is_empty());
        let added = BASE.replace(
            "Color color = 3;",
            "Color color = 3; optional string extra = 4;",
        ) + "message Other { uint64 x = 1; }";
        let added = added.replace("COLOR_RED = 1;", "COLOR_RED = 1; COLOR_BLUE = 2;");
        assert_eq!(check(&added), Vec::<String>::new());
    }

    #[test]
    fn renaming_a_field_is_breaking() {
        let r = check(&BASE.replace("string name = 1", "string title = 1"));
        assert_eq!(r, vec!["agora.v1.Thing: field name renamed to title"]);
    }

    #[test]
    fn changing_type_or_cardinality_is_breaking() {
        assert_eq!(
            check(&BASE.replace("string name = 1", "bytes name = 1")).len(),
            1
        );
        assert_eq!(
            check(&BASE.replace("repeated uint32 ids", "uint32 ids")).len(),
            1
        );
    }

    #[test]
    fn removing_things_is_breaking() {
        assert!(check(&BASE.replace("string name = 1;", ""))
            .iter()
            .any(|m| m.contains("removed")));
        assert!(check(&BASE.replace("message Thing", "message Renamed"))
            .iter()
            .any(|m| m.contains("message agora.v1.Thing was removed")));
        assert!(check(&BASE.replace("COLOR_RED = 1;", ""))
            .iter()
            .any(|m| m.contains("COLOR_RED = 1 was removed")));
    }

    #[test]
    fn renumbering_a_field_is_breaking() {
        let r = check(&BASE.replace("string name = 1", "string name = 9"));
        assert!(r.iter().any(|m| m.contains("name = 1 was removed")));
    }
}
