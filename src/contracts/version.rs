pub const CONTRACT_VERSION: &str = "v2";
pub const EVOLUTION_OS_CONTRACT_VERSION: &str = "evolution-os/v1";
pub const SANDBOX_CONTRACT_VERSION: &str = "sandbox/v2";
pub const STORAGE_CONTRACT_VERSION: &str = "storage/v3";
pub const SIGNAL_CONTRACT_VERSION: &str = "signal/v1";
pub const CLI_FRONTEND_CONTRACT_VERSION: &str = "cli-frontend/v1";
pub const ARTIFACT_DELIVERY_CONTRACT_VERSION: &str = "artifact-delivery/v1";
pub const RELATION_CONTRACT_VERSION: &str = "relation/v1";
pub const CODE_HARNESS_CONTRACT_VERSION: &str = "code-harness/v1";
pub const CODE_EXECUTION_LOOP_CONTRACT_VERSION: &str = "code-execution-loop/v2";
pub const REPO_CONTEXT_BUNDLE_CONTRACT_VERSION: &str = "repo-context/v2";

pub fn evolution_os_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == EVOLUTION_OS_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("evolution-os/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

pub fn sandbox_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == SANDBOX_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("sandbox/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(2))
}

pub fn storage_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == STORAGE_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("storage/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(3))
}

pub fn signal_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == SIGNAL_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("signal/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

pub fn cli_frontend_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == CLI_FRONTEND_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("cli-frontend/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

pub fn artifact_delivery_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == ARTIFACT_DELIVERY_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("artifact-delivery/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

pub fn relation_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == RELATION_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("relation/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

pub fn code_harness_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == CODE_HARNESS_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("code-harness/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

pub fn code_execution_loop_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == CODE_EXECUTION_LOOP_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("code-execution-loop/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(2))
}

pub fn repo_context_bundle_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == REPO_CONTEXT_BUNDLE_CONTRACT_VERSION || normalized == CODE_HARNESS_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("repo-context/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evolution_os_compat_accepts_v1_series() {
        assert!(evolution_os_contract_compatible("evolution-os/v1"));
        assert!(evolution_os_contract_compatible("evolution-os/v1.0"));
        assert!(evolution_os_contract_compatible("EVOLUTION-OS/V1-beta"));
    }

    #[test]
    fn evolution_os_compat_rejects_other_majors() {
        assert!(!evolution_os_contract_compatible("evolution-os/v2"));
        assert!(!evolution_os_contract_compatible("v1"));
        assert!(!evolution_os_contract_compatible("transport-session-event/v2"));
    }

    #[test]
    fn sandbox_compat_accepts_v2_series() {
        assert!(sandbox_contract_compatible("sandbox/v2"));
        assert!(sandbox_contract_compatible("sandbox/v2.1"));
        assert!(sandbox_contract_compatible("SANDBOX/V2-beta"));
    }

    #[test]
    fn sandbox_compat_rejects_other_majors() {
        assert!(!sandbox_contract_compatible("sandbox/v1"));
        assert!(!sandbox_contract_compatible("sandbox/v3"));
        assert!(!sandbox_contract_compatible("v2"));
    }

    #[test]
    fn storage_compat_accepts_v3_series() {
        assert!(storage_contract_compatible("storage/v3"));
        assert!(storage_contract_compatible("storage/v3.2"));
        assert!(storage_contract_compatible("STORAGE/V3-beta"));
    }

    #[test]
    fn storage_compat_rejects_other_majors() {
        assert!(!storage_contract_compatible("storage/v2"));
        assert!(!storage_contract_compatible("storage/v4"));
        assert!(!storage_contract_compatible("v3"));
    }

    #[test]
    fn signal_compat_accepts_v1_series() {
        assert!(signal_contract_compatible("signal/v1"));
        assert!(signal_contract_compatible("signal/v1.2"));
        assert!(signal_contract_compatible("SIGNAL/V1-beta"));
    }

    #[test]
    fn signal_compat_rejects_other_majors() {
        assert!(!signal_contract_compatible("signal/v0"));
        assert!(!signal_contract_compatible("signal/v2"));
        assert!(!signal_contract_compatible("v1"));
    }

    #[test]
    fn cli_frontend_compat_accepts_v1_series() {
        assert!(cli_frontend_contract_compatible("cli-frontend/v1"));
        assert!(cli_frontend_contract_compatible("cli-frontend/v1.1"));
        assert!(cli_frontend_contract_compatible("CLI-FRONTEND/V1-beta"));
    }

    #[test]
    fn cli_frontend_compat_rejects_other_majors() {
        assert!(!cli_frontend_contract_compatible("cli-frontend/v2"));
        assert!(!cli_frontend_contract_compatible("v1"));
        assert!(!cli_frontend_contract_compatible("transport-session-event/v2"));
    }

    #[test]
    fn artifact_delivery_compat_accepts_v1_series() {
        assert!(artifact_delivery_contract_compatible("artifact-delivery/v1"));
        assert!(artifact_delivery_contract_compatible("artifact-delivery/v1.2"));
        assert!(artifact_delivery_contract_compatible("ARTIFACT-DELIVERY/V1-beta"));
    }

    #[test]
    fn artifact_delivery_compat_rejects_other_majors() {
        assert!(!artifact_delivery_contract_compatible("artifact-delivery/v2"));
        assert!(!artifact_delivery_contract_compatible("v1"));
    }

    #[test]
    fn relation_compat_accepts_v1_series() {
        assert!(relation_contract_compatible("relation/v1"));
        assert!(relation_contract_compatible("relation/v1.2"));
        assert!(relation_contract_compatible("RELATION/V1-beta"));
    }

    #[test]
    fn relation_compat_rejects_other_majors() {
        assert!(!relation_contract_compatible("relation/v2"));
        assert!(!relation_contract_compatible("v1"));
    }

    #[test]
    fn code_harness_compat_accepts_v1_series() {
        assert!(code_harness_contract_compatible("code-harness/v1"));
        assert!(code_harness_contract_compatible("code-harness/v1.1"));
        assert!(code_harness_contract_compatible("CODE-HARNESS/V1-beta"));
    }

    #[test]
    fn code_harness_compat_rejects_other_majors() {
        assert!(!code_harness_contract_compatible("code-harness/v2"));
        assert!(!code_harness_contract_compatible("v1"));
    }

    #[test]
    fn code_execution_loop_compat_accepts_v2_series() {
        assert!(code_execution_loop_contract_compatible(
            "code-execution-loop/v2"
        ));
        assert!(code_execution_loop_contract_compatible(
            "code-execution-loop/v2.2"
        ));
        assert!(code_execution_loop_contract_compatible(
            "CODE-EXECUTION-LOOP/V2-beta"
        ));
    }

    #[test]
    fn code_execution_loop_compat_rejects_other_majors() {
        assert!(!code_execution_loop_contract_compatible(
            "code-execution-loop/v1"
        ));
        assert!(!code_execution_loop_contract_compatible(
            "code-execution-loop/v3"
        ));
        assert!(!code_execution_loop_contract_compatible("v2"));
    }

    #[test]
    fn repo_context_bundle_compat_accepts_v2_and_legacy_v1() {
        assert!(repo_context_bundle_contract_compatible("repo-context/v2"));
        assert!(repo_context_bundle_contract_compatible("repo-context/v2.1"));
        assert!(repo_context_bundle_contract_compatible("REPO-CONTEXT/V2-beta"));
        assert!(repo_context_bundle_contract_compatible("code-harness/v1"));
    }

    #[test]
    fn repo_context_bundle_compat_rejects_other_majors() {
        assert!(!repo_context_bundle_contract_compatible("repo-context/v1"));
        assert!(!repo_context_bundle_contract_compatible("repo-context/v3"));
        assert!(!repo_context_bundle_contract_compatible("v2"));
    }
}
