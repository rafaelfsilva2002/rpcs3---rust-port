//! `rpcs3-vfs-paths` — PS3 VFS path canonicalization.
//!
//! Ports `lv2_fs_object::get_path_root_and_trail` from
//! `rpcs3/Emu/Cell/lv2/sys_fs.cpp:298-381`.
//!
//! ## Rules (observed from the C++ reference)
//!
//! * Empty input → `("", "ENOENT")` sentinel.
//! * Consecutive leading `/` with no content → `("", "ENOENT")`.
//! * Path is split on `/`. Components are processed in order:
//!   * `.` — skipped, depth unchanged.
//!   * `..` — pops one component from the trail (or resets root if at
//!     depth 1). Popping past depth 0 returns `("", "ENOENT")`.
//!   * Anything else — assigned to `root` if depth is 0, else appended
//!     to the `trail`.
//!
//! Trailing slashes in the input have no observable effect.
//!
//! Matches the contract tests in `rpcs3/tests/test_sys_fs.cpp`.

/// Result of canonicalization: the mount root device name and the
/// trailing path within it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathParts {
    pub root: String,
    pub trail: String,
}

impl PathParts {
    /// Construct the sentinel `("", "ENOENT")` value returned on
    /// unrecoverable errors.
    #[must_use]
    pub fn enoent() -> Self {
        Self { root: String::new(), trail: "ENOENT".into() }
    }

    #[must_use]
    pub fn is_enoent(&self) -> bool {
        self.root.is_empty() && self.trail == "ENOENT"
    }
}

/// Canonicalize a PS3 filesystem path into `(root_device, trail)`.
///
/// Mirrors `lv2_fs_object::get_path_root_and_trail`. Any input that
/// cannot be satisfied (empty, escapes below root, etc.) returns
/// [`PathParts::enoent`].
#[must_use]
pub fn get_path_root_and_trail(filename: &str) -> PathParts {
    if filename.is_empty() {
        return PathParts::enoent();
    }

    let bytes = filename.as_bytes();
    let mut root = String::new();
    let mut trail = String::new();
    let mut level: usize = 0;
    let mut pos: usize = 0;

    while pos <= bytes.len() {
        // Skip leading `/`s.
        let ndl_pos = bytes[pos..].iter().position(|&b| b != b'/').map(|i| i + pos);

        // Matches C++: `ndl_pos == pos` means the input starts with a
        // non-slash after a position we expected a slash — meaning no
        // delimiter between the previous segment and this one. Only
        // valid condition is `pos > 0`; if `pos == 0` it means the path
        // didn't start with /.
        if let Some(ndl) = ndl_pos {
            if ndl == pos {
                return PathParts::enoent();
            }
        }

        let Some(ndl) = ndl_pos else {
            break; // end of string
        };

        // Find next `/`.
        let dl_pos = bytes[ndl..].iter().position(|&b| b == b'/').map(|i| i + ndl);
        let end = dl_pos.unwrap_or(bytes.len());
        let component = &bytes[ndl..end];

        if component == b"." {
            // no-op on both root/trail AND on the level counter — matches C++:
            // the `continue` jumps over both the tail assignment and level++.
            pos = end;
            continue;
        } else if component == b".." {
            match level {
                0 => return PathParts::enoent(),
                1 => {
                    root.clear();
                }
                _ => {
                    // Drop last component from trail.
                    // C++ uses: trail.resize(trail.find_last_of("/") + 1);
                    //           trail.resize(trail.find_last_not_of("/") + 1);
                    // i.e. cut up to and including the last "/",
                    // then trim trailing slashes.
                    if let Some(last_slash) = trail.rfind('/') {
                        trail.truncate(last_slash);
                        while trail.ends_with('/') {
                            trail.pop();
                        }
                    } else {
                        trail.clear();
                    }
                }
            }
            level -= 1;
            pos = end;
            continue;
        } else {
            // Ordinary component
            if level == 0 {
                root = std::str::from_utf8(component).unwrap_or("").to_owned();
            } else if trail.is_empty() {
                trail = std::str::from_utf8(component).unwrap_or("").to_owned();
            } else {
                trail.push('/');
                trail.push_str(std::str::from_utf8(component).unwrap_or(""));
            }
        }

        level += 1;
        pos = end;
    }

    PathParts { root, trail }
}

// ---------------------------------------------------------------------
// Tests — mirror the expected behaviour from the C++ reference and from
// `rpcs3/tests/test_sys_fs.cpp` as closely as practical.
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn expect(path: &str, root: &str, trail: &str) {
        let got = get_path_root_and_trail(path);
        assert_eq!(
            got,
            PathParts { root: root.to_owned(), trail: trail.to_owned() },
            "divergence for input {path:?}"
        );
    }

    // -- From rpcs3/tests/test_sys_fs.cpp ----------------------------

    #[test]
    fn slash_dot() {
        // `/. ` in C++ test: root empty, trail empty
        expect("/.", "", "");
    }

    #[test]
    fn slash_dots_then_dev() {
        expect("/./././dev_bdvd/./", "dev_bdvd", "");
    }

    #[test]
    fn dotdot_at_root_is_enoent() {
        expect("/../", "", "ENOENT");
    }

    #[test]
    fn plain_dev_hdd0() {
        expect("/dev_hdd0/", "dev_hdd0", "");
    }

    #[test]
    fn dev_hdd0_with_game_subdir() {
        expect("/dev_hdd0/game", "dev_hdd0", "game");
    }

    #[test]
    fn dev_hdd0_with_nested_title() {
        expect("/dev_hdd0/game/NP1234567", "dev_hdd0", "game/NP1234567");
    }

    #[test]
    fn dev_hdd0_with_dotdot_swap() {
        // From test_sys_fs.cpp: `/dev_hdd0/game/NP1234567/../../NP1234568/.`
        // Expected: root=dev_hdd0, trail=NP1234568
        expect(
            "/dev_hdd0/game/NP1234567/../../NP1234568/.",
            "dev_hdd0",
            "NP1234568",
        );
    }

    // -- Empty / trivial --------------------------------------------

    #[test]
    fn empty_is_enoent() {
        let p = get_path_root_and_trail("");
        assert!(p.is_enoent());
    }

    #[test]
    fn single_slash_is_empty_root_and_trail() {
        expect("/", "", "");
    }

    // -- Multiple components ----------------------------------------

    #[test]
    fn three_levels_deep() {
        expect(
            "/dev_hdd0/game/BLES01234/USRDIR",
            "dev_hdd0",
            "game/BLES01234/USRDIR",
        );
    }

    #[test]
    fn trailing_slash_is_ignored() {
        expect("/dev_hdd0/game/", "dev_hdd0", "game");
    }

    // -- `..` escapes -----------------------------------------------

    #[test]
    fn dotdot_pops_last_component() {
        expect("/dev_hdd0/game/..", "dev_hdd0", "");
    }

    #[test]
    fn dotdot_resets_root_at_depth_1() {
        // `/dev_hdd0/..` leaves root empty, trail empty.
        expect("/dev_hdd0/..", "", "");
    }

    #[test]
    fn dotdot_escape_past_root_is_enoent() {
        // `/dev_hdd0/../..` goes below depth 0 → ENOENT.
        expect("/dev_hdd0/../..", "", "ENOENT");
    }

    #[test]
    fn multiple_slashes_between_components_are_treated_as_one() {
        // The C++ reference happily collapses `//` into a single separator
        // (via find_first_not_of). We mirror that.
        expect("/dev_hdd0//game", "dev_hdd0", "game");
        expect("/dev_hdd0///game", "dev_hdd0", "game");
    }

    // -- Dot components (`.`) ---------------------------------------

    #[test]
    fn dot_in_middle_is_noop() {
        expect("/dev_hdd0/./game/./BLES01234", "dev_hdd0", "game/BLES01234");
    }

    #[test]
    fn enoent_sentinel_detection() {
        assert!(PathParts::enoent().is_enoent());
        assert!(!PathParts { root: "dev_hdd0".into(), trail: String::new() }.is_enoent());
    }
}
