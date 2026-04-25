//! `rpcs3-version` — Rust port of `rpcs3/rpcs3_version.cpp`.
//!
//! Version string parsing + branch detection.
//! The C++ build system provides `RPCS3_GIT_BRANCH`,
//! `RPCS3_GIT_FULL_BRANCH`, `RPCS3_GIT_VERSION` as preprocessor defines.
//! Here we accept them as runtime inputs so the crate stays build-free.
//!
//! Frozen:
//!
//! - Version: major=0, minor=0, patch=40, type=Alpha, pre=1 (cpp:29).
//! - `get_commit_and_hash("<commit>-<hash>")` → splits on "-", fallback
//!   `("0", "00000000")` if != 2 parts (cpp:18..25).
//! - Master vs non-master branch logic (cpp:36..48).
//! - `is_release_build`: full branch == `"RPCS3/rpcs3/master"` (cpp:62).
//! - `is_local_build`: full branch == `"local_build"` (cpp:68).

/// Current RPCS3 version constants baked into cpp:29.
pub const VERSION_MAJOR: u32 = 0;
pub const VERSION_MINOR: u32 = 0;
pub const VERSION_PATCH: u32 = 40;
pub const VERSION_PRE: u32 = 1;

/// The release-channel full branch identifier (cpp:62).
pub const RELEASE_FULL_BRANCH: &str = "RPCS3/rpcs3/master";

/// The local-build full branch sentinel (cpp:68).
pub const LOCAL_BUILD_SENTINEL: &str = "local_build";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionType {
    Alpha,
    Beta,
    Rc,
    Release,
}

impl VersionType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alpha => "alpha",
            Self::Beta => "beta",
            Self::Rc => "rc",
            Self::Release => "",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub version_type: VersionType,
    pub pre_release: u32,
}

impl Version {
    /// RPCS3's current version (cpp:29 — 0.0.40 alpha 1).
    #[must_use]
    pub const fn current() -> Self {
        Self {
            major: VERSION_MAJOR,
            minor: VERSION_MINOR,
            patch: VERSION_PATCH,
            version_type: VersionType::Alpha,
            pre_release: VERSION_PRE,
        }
    }

    /// Format as `"major.minor.patch-type-pre"` (roughly cpp
    /// `utils::version::to_string`).
    #[must_use]
    pub fn to_string(&self) -> String {
        match self.version_type {
            VersionType::Release => format!("{}.{}.{}", self.major, self.minor, self.patch),
            other => format!(
                "{}.{}.{}-{}-{}",
                self.major, self.minor, self.patch, other.as_str(), self.pre_release
            ),
        }
    }
}

/// `get_commit_and_hash(git_version)` (cpp:18..25).
/// Returns the pair or the fallback `("0", "00000000")` if `git_version`
/// doesn't have exactly one `-`.
#[must_use]
pub fn get_commit_and_hash(git_version: &str) -> (String, String) {
    let parts: Vec<&str> = git_version.split('-').collect();
    if parts.len() != 2 {
        return ("0".to_string(), "00000000".to_string());
    }
    (parts[0].to_string(), parts[1].to_string())
}

/// `get_version_and_branch(branch, version_str)` (cpp:33..48).
/// On `master` or `HEAD` branches, strips the trailing `-buildnumber` /
/// `-commithash` from the version string. Otherwise returns the verbose
/// format `"<version> | <branch>"`.
#[must_use]
pub fn get_version_and_branch(branch: &str, version_str: &str, full_branch: &str) -> String {
    if branch != "master" && branch != "HEAD" {
        // Verbose path: include branch + local_build marker.
        let mut out = format!("{version_str} | {branch}");
        if is_local_build(full_branch) {
            out.push_str(" | local_build");
        }
        return out;
    }

    // On master/HEAD: strip everything after the last '-'.
    match version_str.rfind('-') {
        Some(idx) => version_str[..idx].to_string(),
        None => version_str.to_string(),
    }
}

/// `is_release_build(full_branch)` (cpp:60..64).
#[must_use]
pub fn is_release_build(full_branch: &str) -> bool {
    full_branch == RELEASE_FULL_BRANCH
}

/// `is_local_build(full_branch)` (cpp:66..70).
#[must_use]
pub fn is_local_build(full_branch: &str) -> bool {
    full_branch == LOCAL_BUILD_SENTINEL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_constants() {
        let v = Version::current();
        assert_eq!(v.major, 0);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 40);
        assert_eq!(v.version_type, VersionType::Alpha);
        assert_eq!(v.pre_release, 1);
    }

    #[test]
    fn version_to_string_alpha() {
        let v = Version::current();
        assert_eq!(v.to_string(), "0.0.40-alpha-1");
    }

    #[test]
    fn version_to_string_release_omits_suffix() {
        let v = Version {
            version_type: VersionType::Release,
            ..Version::current()
        };
        assert_eq!(v.to_string(), "0.0.40");
    }

    #[test]
    fn get_commit_and_hash_well_formed() {
        let (commit, hash) = get_commit_and_hash("1234-abcdef");
        assert_eq!(commit, "1234");
        assert_eq!(hash, "abcdef");
    }

    #[test]
    fn get_commit_and_hash_fallback_on_bad_input() {
        // No dash.
        assert_eq!(get_commit_and_hash("nodash"), ("0".into(), "00000000".into()));
        // More than one dash.
        assert_eq!(get_commit_and_hash("a-b-c"), ("0".into(), "00000000".into()));
        // Empty.
        assert_eq!(get_commit_and_hash(""), ("0".into(), "00000000".into()));
    }

    #[test]
    fn get_version_and_branch_master_strips_trailing_dash() {
        assert_eq!(
            get_version_and_branch("master", "0.0.40-1234-abcdef", "RPCS3/rpcs3/master"),
            "0.0.40-1234"
        );
    }

    #[test]
    fn get_version_and_branch_head_strips_trailing_dash() {
        assert_eq!(
            get_version_and_branch("HEAD", "0.0.40-1234-abcdef", "RPCS3/rpcs3/master"),
            "0.0.40-1234"
        );
    }

    #[test]
    fn get_version_and_branch_non_master_includes_branch() {
        assert_eq!(
            get_version_and_branch("feature/new", "0.0.40-1234", "RPCS3/rpcs3/feature/new"),
            "0.0.40-1234 | feature/new"
        );
    }

    #[test]
    fn get_version_and_branch_local_build_flag() {
        assert_eq!(
            get_version_and_branch("dev", "0.0.40", "local_build"),
            "0.0.40 | dev | local_build"
        );
    }

    #[test]
    fn release_and_local_detection() {
        assert!(is_release_build("RPCS3/rpcs3/master"));
        assert!(!is_release_build("RPCS3/rpcs3/dev"));
        assert!(!is_release_build(""));

        assert!(is_local_build("local_build"));
        assert!(!is_local_build("RPCS3/rpcs3/master"));
    }

    #[test]
    fn version_type_as_str() {
        assert_eq!(VersionType::Alpha.as_str(), "alpha");
        assert_eq!(VersionType::Beta.as_str(), "beta");
        assert_eq!(VersionType::Rc.as_str(), "rc");
        assert_eq!(VersionType::Release.as_str(), "");
    }
}
