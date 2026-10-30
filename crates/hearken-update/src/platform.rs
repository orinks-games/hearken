//! The platform names releases are published under. `hearken release`
//! names its archives and manifests with these, and a release looks for
//! its own.

/// The platform `os` and `arch` (as `std::env::consts` names them) are
/// released as: `windows-x64`, `linux-arm64`, `macos-arm64`, and `macos`
/// for an Intel Mac.
pub fn name(os: &str, arch: &str) -> String {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    match (os, arch) {
        ("macos", "x64") => "macos".to_string(),
        _ => format!("{os}-{arch}"),
    }
}

/// This program's platform.
pub fn current() -> String {
    name(std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_release_archives() {
        assert_eq!(name("windows", "x86_64"), "windows-x64");
        assert_eq!(name("windows", "aarch64"), "windows-arm64");
        assert_eq!(name("linux", "x86_64"), "linux-x64");
        assert_eq!(name("linux", "aarch64"), "linux-arm64");
        assert_eq!(name("macos", "aarch64"), "macos-arm64");
        assert_eq!(name("macos", "x86_64"), "macos");
        assert!(crate::manifest::is_channel(&current()));
    }
}
