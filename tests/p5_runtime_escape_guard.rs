use std::{fs, path::Path};

#[test]
fn direct_tool_provider_calls_outside_runtime_are_blocked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    scan_rs_files(&root, &mut |path, line_no, line| {
        let normalized = path.replace('\\', "/");
        let allow = normalized.ends_with("/providers/mod.rs")
            || normalized.ends_with("/runtime/mod.rs")
            || normalized.ends_with("/tools/mod.rs")
            || normalized.ends_with("/tools/cli_forge.rs");
        if allow {
            return;
        }
        let trimmed = line.trim();
        if trimmed.contains(".providers.chat(")
            || trimmed.contains(".providers.chat_with_policy(")
            || trimmed.contains(".tools.execute(")
        {
            violations.push(format!("{path}:{line_no}: {trimmed}"));
        }
    });

    assert!(
        violations.is_empty(),
        "Runtime escape detected (direct provider/tool call outside runtime):\n{}",
        violations.join("\n")
    );
}

fn scan_rs_files(dir: &Path, visitor: &mut dyn FnMut(String, usize, String)) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_rs_files(&path, visitor);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                if let Ok(content) = fs::read_to_string(&path) {
                    for (idx, line) in content.lines().enumerate() {
                        visitor(path.display().to_string(), idx + 1, line.to_string());
                    }
                }
            }
        }
    }
}



