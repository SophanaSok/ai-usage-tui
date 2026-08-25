//! How this copy of `ai-usage-tui` was installed, and how to upgrade it.
//!
//! There are seven install channels — cargo, binstall, Homebrew, Scoop, Chocolatey, the `.deb`
//! and `.rpm`, and `scripts/install.sh` — and until now nothing in the tool knew which one it
//! came from or said a word about upgrading. A user on v0.6.0 had no way to learn that v0.9.0
//! existed, and someone who installed by one route and upgraded by another would end up with two
//! binaries and no idea which was on `PATH`.
//!
//! Two separate questions, kept separate on purpose:
//!
//! - **"How do I upgrade?"** is answered from the running binary's own path. No network, no new
//!   privacy surface, and correct even offline. This is always on.
//! - **"Is there anything to upgrade to?"** needs the network, so it is opt-in and off by
//!   default, exactly as `zen_pricing` is. A tool whose pitch is "reads usage metadata, writes
//!   nothing, transmits nothing" does not get to phone home because it would be convenient.

use std::path::{Path, PathBuf};

/// The repository releases are published from.
pub const RELEASES_URL: &str = "https://github.com/SophanaSok/ai-usage-tui/releases";

/// Where this binary appears to have come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// `cargo install` or `cargo binstall`, into the cargo bin directory.
    Cargo,
    Homebrew,
    Scoop,
    Chocolatey,
    /// The `.deb` or `.rpm`, which install to a system prefix the user does not own.
    SystemPackage,
    /// `scripts/install.sh`, which installs to `~/.local/bin`.
    InstallScript,
    /// A path none of the above explains — a build tree, a manual copy, a distro package we do
    /// not ship. Deliberately not guessed at: naming the wrong upgrade command is worse than
    /// admitting we do not know, because running it can install a *second* copy.
    Unknown,
}

impl Channel {
    /// The exact command to upgrade, or `None` when we cannot say.
    pub fn upgrade_command(self) -> Option<&'static str> {
        match self {
            Channel::Cargo => Some("cargo install ai-usage-tui --locked"),
            Channel::Homebrew => Some("brew upgrade ai-usage-tui"),
            Channel::Scoop => Some("scoop update ai-usage-tui"),
            Channel::Chocolatey => Some("choco upgrade ai-usage-tui"),
            // Not `apt install` or `dnf upgrade`: the .deb and .rpm are attached to a GitHub
            // release, not served from a repository, so there is nothing for a package manager
            // to upgrade *from*. Downloading the new one is the actual answer.
            Channel::SystemPackage => None,
            Channel::InstallScript => {
                Some("curl -fsSL https://raw.githubusercontent.com/SophanaSok/ai-usage-tui/main/scripts/install.sh | sh")
            }
            Channel::Unknown => None,
        }
    }

    /// How to describe where this binary came from, in a sentence.
    pub fn label(self) -> &'static str {
        match self {
            Channel::Cargo => "cargo",
            Channel::Homebrew => "Homebrew",
            Channel::Scoop => "Scoop",
            Channel::Chocolatey => "Chocolatey",
            Channel::SystemPackage => "a system package (.deb/.rpm)",
            Channel::InstallScript => "scripts/install.sh",
            Channel::Unknown => "an unrecognised location",
        }
    }
}

/// Infer the channel from the path a binary is running from.
///
/// Takes the path rather than calling `current_exe` so it is pure, and so the tests can cover
/// every platform's layout from any platform. Matching is on whole path components, never on
/// substrings: `/home/scoopuser/bin` is not a Scoop install, and a project directory called
/// `homebrew-notes` is not Homebrew.
pub fn detect_channel(exe: &Path, home: Option<&Path>) -> Channel {
    // Split on both separators rather than using `Path::components`, which on Unix treats a
    // Windows path as a single component and would silently recognise nothing. The test suite
    // covers every platform's layout from every platform, and this is what lets it: a
    // Linux-only pass would otherwise hide a Windows bug until someone filed it.
    let lowered = exe.to_string_lossy().to_lowercase();
    let components: Vec<&str> = lowered
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    let has = |needle: &str| components.contains(&needle);
    let has_pair = |a: &str, b: &str| components.windows(2).any(|w| w[0] == a && w[1] == b);

    // Homebrew: /opt/homebrew/..., /usr/local/Cellar/..., /home/linuxbrew/.linuxbrew/...
    if has("homebrew") || has("cellar") || has("linuxbrew") || has(".linuxbrew") {
        return Channel::Homebrew;
    }
    if has("scoop") {
        return Channel::Scoop;
    }
    if has("chocolatey") {
        return Channel::Chocolatey;
    }

    // Cargo: $CARGO_HOME/bin, or ~/.cargo/bin. Checked before the generic home cases below,
    // because ~/.cargo/bin is also under the home directory.
    if has_pair(".cargo", "bin") || has_pair("cargo", "bin") {
        return Channel::Cargo;
    }

    // install.sh's destination. Checked against the real home when we have one, so that a path
    // like /opt/other/.local/bin is not mistaken for it.
    if let Some(home) = home {
        if exe.starts_with(home.join(".local").join("bin")) {
            return Channel::InstallScript;
        }
        if exe.starts_with(home.join(".cargo").join("bin")) {
            return Channel::Cargo;
        }
    }

    // The .deb and .rpm both install to /usr/bin. /usr/local/bin is where a hand-copied binary
    // usually lands, so it is deliberately *not* treated as a package.
    if has_pair("usr", "bin") && !has("local") {
        return Channel::SystemPackage;
    }

    Channel::Unknown
}

/// The running binary's channel, or `Unknown` if its own path cannot be resolved.
pub fn current_channel() -> (Option<PathBuf>, Channel) {
    let exe = std::env::current_exe().ok();
    let home = crate::utils::home_dir();
    match &exe {
        Some(path) => {
            let channel = detect_channel(path, home.as_deref());
            (exe.clone(), channel)
        }
        None => (None, Channel::Unknown),
    }
}

/// A release version as three numbers, for comparison. `None` if it does not look like one.
///
/// Deliberately not a semver dependency: this compares two strings the project itself produces,
/// and the whole of the parsing is below. Pre-release and build metadata are ignored rather than
/// ordered, because this project has never published either and guessing at an ordering it does
/// not use would be inventing a rule.
pub fn parse_version(text: &str) -> Option<[u32; 3]> {
    let trimmed = text.trim().trim_start_matches('v');
    let core = trimmed.split(['-', '+']).next().unwrap_or(trimmed);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some([major, minor, patch])
}

/// Whether `latest` is newer than `current`. `false` when either is unparseable — an unreadable
/// tag is not evidence of an upgrade.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// Ask GitHub for the tag of the latest release.
///
/// The only network call in this module, reached only from `--doctor` and only when
/// `[update] check = true`. Short timeout and no retries: this is a convenience, and a slow or
/// unreachable network must not make a diagnostic command hang.
///
/// Sends no usage data, no identifiers and no query parameters -- it is a plain GET of a public
/// endpoint. The User-Agent is required by GitHub's API and names the tool and nothing else.
pub fn latest_release_tag() -> anyhow::Result<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }

    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent(concat!("ai-usage-tui/", env!("CARGO_PKG_VERSION")))
        .build()?
        .get("https://api.github.com/repos/SophanaSok/ai-usage-tui/releases/latest")
        .send()?
        .error_for_status()?;
    Ok(response.json::<Release>()?.tag_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn each_channel_is_recognised_from_its_own_layout() {
        let home = p("/home/dev");
        for (path, expected) in [
            ("/home/dev/.cargo/bin/ai-usage-tui", Channel::Cargo),
            ("/opt/homebrew/bin/ai-usage-tui", Channel::Homebrew),
            (
                "/usr/local/Cellar/ai-usage-tui/0.9.0/bin/ai-usage-tui",
                Channel::Homebrew,
            ),
            (
                "/home/linuxbrew/.linuxbrew/bin/ai-usage-tui",
                Channel::Homebrew,
            ),
            (
                "C:\\Users\\dev\\scoop\\shims\\ai-usage-tui.exe",
                Channel::Scoop,
            ),
            (
                "C:\\ProgramData\\chocolatey\\bin\\ai-usage-tui.exe",
                Channel::Chocolatey,
            ),
            ("/usr/bin/ai-usage-tui", Channel::SystemPackage),
            ("/home/dev/.local/bin/ai-usage-tui", Channel::InstallScript),
        ] {
            assert_eq!(
                detect_channel(&p(path), Some(&home)),
                expected,
                "{path} was not recognised as {expected:?}"
            );
        }
    }

    /// Guessing wrong is worse than saying nothing: the suggested command would install a
    /// *second* copy somewhere else on `PATH`, and the user would be upgrading a binary they are
    /// not running.
    #[test]
    fn an_unrecognised_location_is_not_guessed_at() {
        let home = p("/home/dev");
        for path in [
            "/home/dev/Projects/ai-usage-tui/target/release/ai-usage-tui",
            "/usr/local/bin/ai-usage-tui",
            "/opt/custom/ai-usage-tui",
        ] {
            let channel = detect_channel(&p(path), Some(&home));
            assert_eq!(channel, Channel::Unknown, "{path} was guessed at");
            assert!(channel.upgrade_command().is_none());
        }
    }

    /// Matching on whole components, not substrings.
    #[test]
    fn a_lookalike_directory_is_not_a_channel() {
        let home = p("/home/dev");
        assert_eq!(
            detect_channel(&p("/home/scoopuser/bin/ai-usage-tui"), Some(&home)),
            Channel::Unknown
        );
        assert_eq!(
            detect_channel(&p("/home/dev/homebrew-notes/ai-usage-tui"), Some(&home)),
            Channel::Unknown
        );
    }

    #[test]
    fn versions_compare_numerically_not_lexically() {
        // "0.9.0" > "0.10.0" as strings, which is the bug this exists to not have.
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(!is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.9.0", "0.9.0"));
        assert!(is_newer("0.9.0", "v0.9.1"));
        assert!(is_newer("0.9.0", "1.0.0"));
    }

    /// An unreadable tag is not evidence of an upgrade. GitHub's "latest release" is whatever
    /// the repository owner marked as such, and a tag like `nightly` must not be reported as
    /// newer than the version in hand.
    #[test]
    fn an_unparseable_version_never_reads_as_newer() {
        assert!(!is_newer("0.9.0", "nightly"));
        assert!(!is_newer("0.9.0", ""));
        assert!(!is_newer("0.9.0", "1.2.3.4"));
        assert!(!is_newer("not-a-version", "1.0.0"));
    }

    #[test]
    fn every_named_channel_can_say_how_to_upgrade_or_admits_it_cannot() {
        for channel in [
            Channel::Cargo,
            Channel::Homebrew,
            Channel::Scoop,
            Channel::Chocolatey,
            Channel::InstallScript,
        ] {
            assert!(
                channel.upgrade_command().is_some(),
                "{channel:?} has no upgrade command"
            );
        }
        // Both of these deliberately have none; the caller points at the releases page.
        assert!(Channel::SystemPackage.upgrade_command().is_none());
        assert!(Channel::Unknown.upgrade_command().is_none());
    }
}
