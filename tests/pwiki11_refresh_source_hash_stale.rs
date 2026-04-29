use std::fs;

use autoloop::plugins::gitmemory_core::{
    hot_index_updater::{HotIndexUpdater, RefreshPlanMode},
    incremental_compiler::source_digest,
};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[test]
fn pwiki11_source_hash_change_marks_file_stale() {
    let temp = std::env::temp_dir().join(format!("pwiki11_source_hash_{}", now_ms()));
    fs::create_dir_all(temp.join(".gitmemory")).expect("mkdir gitmemory");
    fs::create_dir_all(temp.join("docs")).expect("mkdir docs");

    let source_file = "docs/a.md";
    fs::write(temp.join(source_file), "# A\nfirst\n").expect("write first");
    let digest = source_digest(&temp, source_file)
        .expect("digest")
        .expect("digest exists");

    fs::write(
        temp.join(".gitmemory").join("hot_index.json"),
        serde_json::to_string_pretty(&vec![serde_json::json!({
            "source_file": source_file,
            "source_digest": digest,
            "summary": "ok",
            "updated_at_ms": 1
        })])
        .expect("serialize index"),
    )
    .expect("write index");

    fs::write(temp.join(source_file), "# A\nsecond\n").expect("update source");
    let plan = HotIndexUpdater::plan_refresh(&temp, &[], RefreshPlanMode::Detect).expect("plan");

    assert!(plan.stale_files.iter().any(|item| item == source_file));
    let _ = fs::remove_dir_all(&temp);
}



