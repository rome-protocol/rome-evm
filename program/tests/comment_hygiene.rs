//! Production comments state invariants; they do not carry audit-ticket or
//! task IDs (house rule: no historical context or ticket references in code
//! comments). Tests may reference the finding they refute — that traceability
//! is deliberate — so this guard only inspects source BEFORE the first
//! `#[cfg(test)]` in each file under `program/src`.

use std::{fs, path::Path};

fn production_part(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or("")
}

fn visit(dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            visit(&p, out);
            continue;
        }
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            // Test-support modules are compiled only under cfg(test) at their mod site.
            if p.file_name().map(|n| n == "test_support.rs").unwrap_or(false) {
                continue;
            }
            let text = fs::read_to_string(&p).unwrap();
            for (i, line) in production_part(&text).lines().enumerate() {
                let l = line.to_ascii_lowercase();
                if l.contains("halborn") || l.contains("find-0") || l.contains("red-0") {
                    out.push(format!("{}:{}: {}", p.display(), i + 1, line.trim()));
                }
            }
        }
    }
}

#[test]
fn production_comments_carry_no_audit_ticket_ids() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    visit(&root, &mut hits);
    assert!(
        hits.is_empty(),
        "audit-ticket IDs in production comments (state the invariant instead):\n{}",
        hits.join("\n")
    );
}
