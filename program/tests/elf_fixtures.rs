// The vendored ELF snapshots are what makes t22_differential deterministic, and
// what its byte-equality assertions are measured against. Two guards, because
// they catch different things:
//
//   this test  — the vendored bytes still match the manifest (local tampering,
//                a bad merge, a truncated checkout). Hermetic.
//   elf-drift  — the DEPLOYED programs still match the manifest (upstream
//                redeploy). Needs a live cluster, so it is a workflow.
//
// A vendored snapshot whose upstream has moved makes every byte-equality
// assertion in the differential harness a statement about history, not about
// what Rome will actually CPI. That is why the alarm is not optional (DESIGN §4b).
//
// To update intentionally: scripts/sync-elf-fixtures.sh

use std::{collections::BTreeMap, fs, path::PathBuf};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/elf")
}

/// name -> (program_id, sha256) parsed from the manifest.
fn manifest() -> BTreeMap<String, (String, String)> {
    let raw = fs::read_to_string(dir().join("MANIFEST.txt"))
        .expect("tests/fixtures/elf/MANIFEST.txt must exist — it pins what the harness measures against");
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut f = l.split_whitespace();
            let name = f.next().expect("name").to_string();
            let program_id = f.next().expect("program_id").to_string();
            let sha = f.next().expect("sha256").to_string();
            (name, (program_id, sha))
        })
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    // solana_program re-exports the hash used across the repo; sha256 here only
    // needs to be stable and comparable, not cryptographically chosen.
    solana_program::hash::hashv(&[bytes]).to_string()
}

#[test]
fn manifest_covers_every_vendored_elf() {
    let m = manifest();
    for entry in fs::read_dir(dir()).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) == Some("so") {
            let name = p.file_stem().unwrap().to_str().unwrap();
            assert!(
                m.contains_key(name),
                "{name}.so is vendored but absent from MANIFEST.txt — an unpinned snapshot is an unmeasured one"
            );
        }
    }
    assert!(!m.is_empty(), "manifest must not be empty");
}

#[test]
fn vendored_elfs_match_the_manifest() {
    for (name, (_program_id, want)) in manifest() {
        let path = dir().join(format!("{name}.so"));
        let got = sha256_hex(&fs::read(&path).unwrap_or_else(|_| panic!("missing {name}.so")));
        assert_eq!(
            got, want,
            "{name}.so no longer matches MANIFEST.txt. If this is an intentional \
             re-fetch, run scripts/sync-elf-fixtures.sh and re-read the \
             differential harness's expectations before committing"
        );
    }
}
