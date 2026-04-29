use std::fs;

use autoloop::plugins::gitmemory_core::hot_index_updater::{HotIndexEntry, HotIndexUpdater, RefreshPlanMode};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[test]
fn pwiki11_hot_index_refresh_modes_detect_force_page() {
    let temp = std::env::temp_dir().join(format!("pwiki11_refresh_modes_{}", now_ms()));
    fs::create_dir_all(temp.join(".gitmemory")).expect("mkdir gitmemory");
    fs::create_dir_all(temp.join("docs")).expect("mkdir docs");

    fs::write(temp.join("docs").join("a.md"), "# A\n").expect("write a");
    fs::write(temp.join("docs").join("b.md"), "# B\n").expect("write b");

    let index = vec![
        HotIndexEntry {
            source_file: "docs/a.md".to_string(),
            source_digest: "stale:a".to_string(),
            summary: "stale".to_string(),
            updated_at_ms: 1,
        },
        HotIndexEntry {
            source_file: "docs/b.md".to_string(),
            source_digest: "stale:b".to_string(),
            summary: "stale".to_string(),
            updated_at_ms: 1,
        },
    ];
    fs::write(
        temp.join(".gitmemory").join("hot_index.json"),
        serde_json::to_string_pretty(&index).expect("serialize index"),
    )
    .expect("write index");

    let detect = HotIndexUpdater::plan_refresh(&temp, &[], RefreshPlanMode::Detect).expect("detect");
    assert!(!detect.stale_files.is_empty());

    let force = HotIndexUpdater::plan_refresh(&temp, &[], RefreshPlanMode::Force).expect("force");
    assert!(!force.effective_changed_files.is_empty());

    let page = HotIndexUpdater::plan_refresh_with_options(
        &temp,
        &[],
        RefreshPlanMode::Page,
        Some(1),
        Some(1),
    )
    .expect("page");
    assert_eq!(page.paged_files.len(), 1);
    assert_eq!(page.total_candidates, 2);

    let _ = fs::remove_dir_all(&temp);
}



