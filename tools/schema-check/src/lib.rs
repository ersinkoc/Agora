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
    message_syntax: BTreeMap<String, &'a str>,
    enums: BTreeMap<String, &'a EnumDescriptorProto>,
    enum_syntax: BTreeMap<String, &'a str>,
    extensions: BTreeMap<(String, i32), &'a FieldDescriptorProto>,
}

fn index(fds: &FileDescriptorSet) -> Index<'_> {
    fn walk<'a>(prefix: &str, m: &'a DescriptorProto, syntax: &'a str, ix: &mut Index<'a>) {
        let name = format!("{prefix}.{}", m.name());
        for extension in &m.extension {
            ix.extensions.insert(
                (extension.extendee().to_string(), extension.number()),
                extension,
            );
        }
        for n in &m.nested_type {
            walk(&name, n, syntax, ix);
        }
        for e in &m.enum_type {
            let enum_name = format!("{name}.{}", e.name());
            ix.enum_syntax.insert(enum_name.clone(), syntax);
            ix.enums.insert(enum_name, e);
        }
        ix.message_syntax.insert(name.clone(), syntax);
        ix.messages.insert(name, m);
    }
    let mut ix = Index::default();
    for f in &fds.file {
        let pkg = f.package();
        for extension in &f.extension {
            ix.extensions.insert(
                (extension.extendee().to_string(), extension.number()),
                extension,
            );
        }
        for m in &f.message_type {
            walk(pkg, m, f.syntax(), &mut ix);
        }
        for e in &f.enum_type {
            let name = format!("{pkg}.{}", e.name());
            ix.enum_syntax.insert(name.clone(), f.syntax());
            ix.enums.insert(name, e);
        }
    }
    ix
}

fn describe(f: &FieldDescriptorProto) -> String {
    let label = match f.label() {
        Label::Required => "required ",
        Label::Repeated => "repeated ",
        _ if f.proto3_optional() => "optional ",
        _ => "",
    };
    format!("{label}{:?} {}", f.r#type(), f.type_name())
}

fn ranges_cover(base: &[(i64, i64)], head: &[(i64, i64)]) -> bool {
    base.iter().all(|&(start, end)| {
        let mut cursor = start;
        let mut ranges = head.to_vec();
        ranges.sort_unstable();
        for (from, to) in ranges {
            if from > cursor || to <= cursor {
                continue;
            }
            cursor = cursor.max(to);
            if cursor >= end {
                return true;
            }
        }
        cursor >= end
    })
}

fn message_ranges(m: &DescriptorProto) -> Vec<(i64, i64)> {
    m.reserved_range
        .iter()
        .map(|r| (i64::from(r.start()), i64::from(r.end())))
        .collect()
}

fn extension_ranges(m: &DescriptorProto) -> Vec<(i64, i64)> {
    m.extension_range
        .iter()
        .map(|r| (i64::from(r.start()), i64::from(r.end())))
        .collect()
}

fn enum_ranges(e: &EnumDescriptorProto) -> Vec<(i64, i64)> {
    e.reserved_range
        .iter()
        .map(|r| (i64::from(r.start()), i64::from(r.end()) + 1))
        .collect()
}

fn oneof_name<'a>(message: &'a DescriptorProto, field: &FieldDescriptorProto) -> Option<&'a str> {
    let index = usize::try_from(field.oneof_index?).ok()?;
    message.oneof_decl.get(index).map(|oneof| oneof.name())
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
        if h.message_syntax.get(name) != b.message_syntax.get(name) {
            out.push(format!(
                "message {name}: syntax changed from `{}` to `{}`",
                b.message_syntax.get(name).copied().unwrap_or(""),
                h.message_syntax.get(name).copied().unwrap_or("")
            ));
        }
        let base_map = bm.options.as_ref().is_some_and(|o| o.map_entry());
        let head_map = hm.options.as_ref().is_some_and(|o| o.map_entry());
        if base_map != head_map {
            out.push(format!("message {name}: map semantics changed"));
        }
        for reserved in &bm.reserved_name {
            if !hm.reserved_name.contains(reserved) {
                out.push(format!(
                    "message {name}: reserved name `{reserved}` was removed"
                ));
            }
        }
        if !ranges_cover(&message_ranges(bm), &message_ranges(hm)) {
            out.push(format!(
                "message {name}: reserved number range was narrowed"
            ));
        }
        if !ranges_cover(&extension_ranges(bm), &extension_ranges(hm)) {
            out.push(format!("message {name}: extension range was narrowed"));
        }
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
                    } else if hf.json_name() != bf.json_name() {
                        out.push(format!(
                            "{name}.{}: JSON name changed from `{}` to `{}`",
                            bf.name(),
                            bf.json_name(),
                            hf.json_name()
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
                    if hf.default_value != bf.default_value {
                        out.push(format!(
                            "{name}.{}: default value changed from `{:?}` to `{:?}`",
                            bf.name(),
                            bf.default_value,
                            hf.default_value
                        ));
                    }
                    let base_options = bf.options.as_ref();
                    let head_options = hf.options.as_ref();
                    let base_ctype = base_options.and_then(|o| o.ctype).unwrap_or_default();
                    let head_ctype = head_options.and_then(|o| o.ctype).unwrap_or_default();
                    if base_ctype != head_ctype {
                        out.push(format!("{name}.{}: C++ type option changed", bf.name()));
                    }
                    let base_jstype = base_options.and_then(|o| o.jstype).unwrap_or_default();
                    let head_jstype = head_options.and_then(|o| o.jstype).unwrap_or_default();
                    if base_jstype != head_jstype {
                        out.push(format!(
                            "{name}.{}: JavaScript type option changed",
                            bf.name()
                        ));
                    }
                    let (base_oneof, head_oneof) = (oneof_name(bm, bf), oneof_name(hm, hf));
                    if base_oneof != head_oneof {
                        out.push(format!(
                            "{name}.{}: oneof membership changed from `{}` to `{}`",
                            bf.name(),
                            base_oneof.unwrap_or("none"),
                            head_oneof.unwrap_or("none")
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
        if h.enum_syntax.get(name) != b.enum_syntax.get(name) {
            out.push(format!(
                "enum {name}: syntax changed from `{}` to `{}`",
                b.enum_syntax.get(name).copied().unwrap_or(""),
                h.enum_syntax.get(name).copied().unwrap_or("")
            ));
        }
        for reserved in &be.reserved_name {
            if !he.reserved_name.contains(reserved) {
                out.push(format!(
                    "enum {name}: reserved name `{reserved}` was removed"
                ));
            }
        }
        if !ranges_cover(&enum_ranges(be), &enum_ranges(he)) {
            out.push(format!("enum {name}: reserved number range was narrowed"));
        }
        for bv in &be.value {
            if he
                .value
                .iter()
                .any(|v| v.number() == bv.number() && v.name() == bv.name())
            {
                continue;
            }
            match he.value.iter().find(|v| v.number() == bv.number()) {
                None => out.push(format!(
                    "{name}: value {} = {} was removed",
                    bv.name(),
                    bv.number()
                )),
                Some(hv) => {
                    out.push(format!(
                        "{name}: value {} renamed to {}",
                        bv.name(),
                        hv.name()
                    ));
                }
            }
        }
    }
    for ((extendee, number), bf) in &b.extensions {
        match h.extensions.get(&(extendee.clone(), *number)) {
            None => out.push(format!(
                "extension {} = {number} of {extendee} was removed",
                bf.name()
            )),
            Some(hf) => {
                if hf.name() != bf.name() {
                    out.push(format!(
                        "extension {} = {number} of {extendee} renamed to {}",
                        bf.name(),
                        hf.name()
                    ));
                }
                if describe(hf) != describe(bf) {
                    out.push(format!(
                        "extension {} = {number} of {extendee}: type changed from `{}` to `{}`",
                        bf.name(),
                        describe(bf).trim(),
                        describe(hf).trim()
                    ));
                }
                if hf.default_value != bf.default_value {
                    out.push(format!(
                        "extension {} = {number} of {extendee}: default value changed",
                        bf.name()
                    ));
                }
                let base_options = bf.options.as_ref();
                let head_options = hf.options.as_ref();
                let base_ctype = base_options.and_then(|o| o.ctype).unwrap_or_default();
                let head_ctype = head_options.and_then(|o| o.ctype).unwrap_or_default();
                if base_ctype != head_ctype {
                    out.push(format!(
                        "extension {} = {number} of {extendee}: C++ type option changed",
                        bf.name()
                    ));
                }
                let base_jstype = base_options.and_then(|o| o.jstype).unwrap_or_default();
                let head_jstype = head_options.and_then(|o| o.jstype).unwrap_or_default();
                if base_jstype != head_jstype {
                    out.push(format!(
                        "extension {} = {number} of {extendee}: JavaScript type option changed",
                        bf.name()
                    ));
                }
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
