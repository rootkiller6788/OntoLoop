use std::fs;

use autoloop::plugins::gitmemory_core::ingest_validator::{IngestValidationMode, IngestValidator};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[test]
fn pwiki11_ingest_validate_only_reports_broken_and_unindexed() {
    let temp = std::env::temp_dir().join(format!("pwiki11_ingest_validate_{}", now_ms()));
    fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
    fs::write(temp.join("docs").join("indexed.md"), "# Indexed\n").expect("write indexed");
    fs::write(temp.join("docs").join("unindexed.md"), "# Unindexed\n").expect("write unindexed");

    for plane in ["graph", "vector", "search"] {
        let dir = temp.join(".gitmemory").join("projections").join(plane);
        fs::create_dir_all(&dir).expect("mkdir projection dir");
        fs::write(dir.join("docs_indexed_md.json"), "{}")
            .expect("write projection");
    }

    let report = IngestValidator::validate(
        &temp,
        "canonical/new.md",
        "[[docs/indexed]] [[docs/unindexed]] [[docs/missing]]",
        IngestValidationMode::ValidateOnly,
    )
    .expect("validate");

    assert!(!report.passed);
    assert!(report.broken_links.iter().any(|x| x == "docs/missing.md"));
    assert!(report.unindexed.iter().any(|x| x == "docs/unindexed.md"));

    let _ = fs::remove_dir_all(&temp);
}



