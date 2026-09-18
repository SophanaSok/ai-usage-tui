use chrono::Datelike;
use std::ffi::OsString;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, path::PathBuf};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn format_clock() -> String {
    chrono::Local::now().format("%H:%M:%S %Z").to_string()
}

/// Start of the current calendar day, in the machine's local timezone.
///
/// Calendar boundaries must be local: a UTC midnight cutoff puts a user's evening work in
/// "tomorrow" and makes the dashboard's daily total disagree with their own sense of the day.
pub fn local_day_start() -> i64 {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .map(|dt| dt.timestamp())
        // A DST transition can make local midnight ambiguous or nonexistent; fall back to a
        // rolling 24h window rather than panicking on one day of the year.
        .unwrap_or_else(|| now.timestamp() - 86_400)
}

/// Start of the current calendar month, in the machine's local timezone.
pub fn local_month_start() -> i64 {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|| now.timestamp() - 30 * 86_400)
}

pub fn format_count(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

/// How the path resolvers below read the environment.
///
/// Production passes [`system_env`]. Tests pass a fixed lookup instead of calling
/// `std::env::set_var`: Cargo runs tests as threads of one process, so setting a variable
/// mutates state every other test shares -- and `set_var` is `unsafe` from edition 2024 onward.
/// This mirrors `collector::billing::Signals`, which injects its environment the same way.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// The real environment.
pub fn system_env(name: &str) -> Option<OsString> {
    env::var_os(name)
}

/// A variable's value, ignoring one that is set but empty.
fn non_empty(env: Env<'_>, name: &str) -> Option<OsString> {
    env(name).filter(|value| !value.is_empty())
}

/// The user's home directory.
///
/// Windows does not set `HOME`. Relying on it alone made every path lookup fail there with
/// "HOME is not set" — on a platform for which this project ships Scoop and Chocolatey
/// packages.
pub fn home_dir() -> Option<PathBuf> {
    home_dir_in(&system_env)
}

/// Whether the user asked for no colour, per <https://no-color.org>: `NO_COLOR` present and not
/// empty, whatever its value.
pub fn no_color_in(env: Env<'_>) -> bool {
    non_empty(env, "NO_COLOR").is_some()
}

/// How many colours the terminal can be trusted to draw.
///
/// The dashboard's palette is 24-bit. It used to be sent as such to every terminal, and one that
/// does not understand the sequence draws whatever it makes of it -- usually the wrong colour,
/// sometimes none. There is no query that answers this reliably, so it is read from what the
/// terminal says about itself, and the answer that needs the least from the terminal wins when
/// it says nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColourDepth {
    /// 24-bit: the palette as designed.
    #[default]
    TrueColour,
    /// The 256-colour cube: each colour's nearest neighbour, close enough to pass at a glance.
    Indexed256,
    /// The terminal's own sixteen. Backgrounds are left to the terminal as well, since its
    /// theme may be light and these were chosen against a dark one.
    Ansi16,
}

impl ColourDepth {
    pub fn label(self) -> &'static str {
        match self {
            Self::TrueColour => "truecolor",
            Self::Indexed256 => "256 colours",
            Self::Ansi16 => "16 colours",
        }
    }
}

/// The colour depth, and the variable it was read from -- `--doctor` prints both, because "why
/// does it look different over ssh" is answered by the second.
///
/// `COLORTERM=truecolor` (or `24bit`) is the de facto declaration; `WT_SESSION` is Windows
/// Terminal, which supports it and does not always say so; a `TERM` ending in `-direct` is
/// terminfo's name for the same thing. `256color` in `TERM` is the middle tier. `ssh` and `sudo`
/// commonly drop `COLORTERM`, which is why the middle tier exists: those sessions keep `TERM`.
pub fn colour_depth_in(env: Env<'_>) -> (ColourDepth, &'static str) {
    // Each variable is named at its `non_empty` call, not passed through a helper: the README's
    // environment table is checked against exactly that call shape.
    let lower = |value: Option<OsString>| {
        value
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    };
    let colorterm = lower(non_empty(env, "COLORTERM"));
    if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        return (ColourDepth::TrueColour, "COLORTERM");
    }
    if non_empty(env, "WT_SESSION").is_some() {
        return (ColourDepth::TrueColour, "WT_SESSION");
    }
    let term = lower(non_empty(env, "TERM"));
    if term.ends_with("-direct") {
        return (ColourDepth::TrueColour, "TERM");
    }
    if term.contains("256color") {
        return (ColourDepth::Indexed256, "TERM");
    }
    (
        ColourDepth::Ansi16,
        "no COLORTERM, and TERM names no colour depth",
    )
}

pub fn home_dir_in(env: Env<'_>) -> Option<PathBuf> {
    if let Some(home) = non_empty(env, "HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = non_empty(env, "USERPROFILE") {
        return Some(PathBuf::from(profile));
    }
    let drive = non_empty(env, "HOMEDRIVE")?;
    let path = non_empty(env, "HOMEPATH")?;
    let mut combined = PathBuf::from(drive);
    combined.push(PathBuf::from(path));
    Some(combined)
}

/// Root for user data, honouring `XDG_DATA_HOME`, then `%LOCALAPPDATA%`, then `~/.local/share`.
pub fn data_root() -> Option<PathBuf> {
    data_root_in(&system_env)
}

pub fn data_root_in(env: Env<'_>) -> Option<PathBuf> {
    if let Some(xdg) = non_empty(env, "XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg));
    }
    if cfg!(windows) {
        if let Some(local) = non_empty(env, "LOCALAPPDATA") {
            return Some(PathBuf::from(local));
        }
    }
    Some(home_dir_in(env)?.join(".local").join("share"))
}

/// Root for user config, honouring `XDG_CONFIG_HOME`, then `%APPDATA%`, then `~/.config`.
pub fn config_root() -> Option<PathBuf> {
    config_root_in(&system_env)
}

pub fn config_root_in(env: Env<'_>) -> Option<PathBuf> {
    if let Some(xdg) = non_empty(env, "XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg));
    }
    if cfg!(windows) {
        if let Some(roaming) = non_empty(env, "APPDATA") {
            return Some(PathBuf::from(roaming));
        }
    }
    Some(home_dir_in(env)?.join(".config"))
}

/// Root for user state, honouring `XDG_STATE_HOME`, then `~/.local/state`. Omarchy keeps its
/// agents-panel records here; this tool only ever reads them.
pub fn state_root() -> Option<PathBuf> {
    state_root_in(&system_env)
}

pub fn state_root_in(env: Env<'_>) -> Option<PathBuf> {
    if let Some(xdg) = non_empty(env, "XDG_STATE_HOME") {
        return Some(PathBuf::from(xdg));
    }
    Some(home_dir_in(env)?.join(".local").join("state"))
}

/// Where Omarchy's agents panel writes one JSON record per agent.
pub fn omarchy_usage_dir() -> Option<PathBuf> {
    omarchy_usage_dir_in(&system_env)
}

pub fn omarchy_usage_dir_in(env: Env<'_>) -> Option<PathBuf> {
    Some(
        state_root_in(env)?
            .join("omarchy")
            .join("agents")
            .join("usage"),
    )
}

pub fn db_path() -> Option<PathBuf> {
    db_path_in(&system_env)
}

pub fn db_path_in(env: Env<'_>) -> Option<PathBuf> {
    if let Some(path) = non_empty(env, "OPENCODE_DB_PATH") {
        return Some(PathBuf::from(path));
    }
    Some(data_root_in(env)?.join("opencode").join("opencode.db"))
}

pub fn data_dir() -> Option<PathBuf> {
    data_dir_in(&system_env)
}

pub fn data_dir_in(env: Env<'_>) -> Option<PathBuf> {
    Some(data_root_in(env)?.join("ai-usage-tui"))
}

pub fn journal_path() -> Option<PathBuf> {
    journal_path_in(&system_env)
}

pub fn journal_path_in(env: Env<'_>) -> Option<PathBuf> {
    if let Some(path) = non_empty(env, "AI_USAGE_JOURNAL_PATH") {
        return Some(PathBuf::from(path));
    }
    Some(data_dir_in(env)?.join("usage.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_is_any_non_empty_value() {
        let unset = |_: &str| None;
        let empty = |_: &str| Some(std::ffi::OsString::new());
        let zero = |name: &str| (name == "NO_COLOR").then(|| std::ffi::OsString::from("0"));
        assert!(!no_color_in(&unset));
        assert!(
            !no_color_in(&empty),
            "no-color.org: an empty value does not count"
        );
        assert!(
            no_color_in(&zero),
            "no-color.org: any non-empty value, even `0`"
        );
    }

    #[test]
    fn colour_depth_is_what_the_terminal_says_and_sixteen_when_it_says_nothing() {
        fn env_of(
            pairs: &'static [(&'static str, &'static str)],
        ) -> impl Fn(&str) -> Option<std::ffi::OsString> {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| std::ffi::OsString::from(value))
            }
        }
        let depth = |pairs| colour_depth_in(&env_of(pairs)).0;
        assert_eq!(
            depth(&[("COLORTERM", "truecolor")]),
            ColourDepth::TrueColour
        );
        assert_eq!(depth(&[("COLORTERM", "24bit")]), ColourDepth::TrueColour);
        assert_eq!(depth(&[("WT_SESSION", "abc")]), ColourDepth::TrueColour);
        assert_eq!(depth(&[("TERM", "xterm-direct")]), ColourDepth::TrueColour);
        // What an ssh session that dropped COLORTERM still has.
        assert_eq!(
            depth(&[("TERM", "xterm-256color")]),
            ColourDepth::Indexed256
        );
        assert_eq!(
            depth(&[("TERM", "tmux-256color"), ("COLORTERM", "")]),
            ColourDepth::Indexed256,
            "an empty COLORTERM declares nothing"
        );
        assert_eq!(depth(&[("TERM", "linux")]), ColourDepth::Ansi16);
        assert_eq!(depth(&[]), ColourDepth::Ansi16);
    }

    #[test]
    fn count_formatting_scales_by_magnitude() {
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_500), "1.5K");
        assert_eq!(format_count(2_500_000), "2.5M");
        assert_eq!(format_count(3_000_000_000), "3.0B");
    }

    #[test]
    fn local_day_start_precedes_now_by_less_than_a_day() {
        let start = local_day_start();
        assert!(start <= now());
        assert!(now() - start < 86_400 + 3_600);
    }

    #[test]
    fn local_month_start_precedes_local_day_start() {
        assert!(local_month_start() <= local_day_start());
    }

    /// A fixed environment, so nothing here touches the process's own.
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let owned: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), OsString::from(*v)))
            .collect();
        move |name: &str| {
            owned
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn env_overrides_take_precedence_over_derived_paths() {
        let env = env_of(&[
            ("HOME", "/home/u"),
            ("OPENCODE_DB_PATH", "/tmp/explicit-opencode.db"),
            ("AI_USAGE_JOURNAL_PATH", "/tmp/explicit-journal.db"),
        ]);
        assert_eq!(
            db_path_in(&env).unwrap(),
            PathBuf::from("/tmp/explicit-opencode.db")
        );
        assert_eq!(
            journal_path_in(&env).unwrap(),
            PathBuf::from("/tmp/explicit-journal.db")
        );

        // Without them, both derive from the data root.
        let bare = env_of(&[("HOME", "/home/u")]);
        assert_eq!(
            db_path_in(&bare).unwrap(),
            PathBuf::from("/home/u/.local/share/opencode/opencode.db")
        );
        assert_eq!(
            journal_path_in(&bare).unwrap(),
            PathBuf::from("/home/u/.local/share/ai-usage-tui/usage.db")
        );
    }

    #[test]
    fn a_variable_that_is_set_but_empty_is_not_a_path() {
        // An exported-but-empty OPENCODE_DB_PATH used to resolve to "", which opens nothing and
        // reports no database rather than falling back to the real default.
        let env = env_of(&[
            ("HOME", "/home/u"),
            ("OPENCODE_DB_PATH", ""),
            ("XDG_DATA_HOME", ""),
        ]);
        assert_eq!(
            db_path_in(&env).unwrap(),
            PathBuf::from("/home/u/.local/share/opencode/opencode.db")
        );
    }

    #[test]
    fn the_omarchy_usage_dir_honours_xdg_state_home() {
        let env = env_of(&[("HOME", "/home/u"), ("XDG_STATE_HOME", "/tmp/xdg-state")]);
        assert_eq!(
            omarchy_usage_dir_in(&env).unwrap(),
            PathBuf::from("/tmp/xdg-state/omarchy/agents/usage")
        );
        let bare = env_of(&[("HOME", "/home/u")]);
        assert_eq!(
            omarchy_usage_dir_in(&bare).unwrap(),
            PathBuf::from("/home/u/.local/state/omarchy/agents/usage")
        );
    }

    /// Windows sets none of `HOME` or the XDG variables; without these fallbacks every path
    /// lookup there failed with "HOME is not set".
    #[test]
    fn the_windows_stand_ins_resolve_a_home() {
        assert_eq!(
            home_dir_in(&env_of(&[("USERPROFILE", "C:\\Users\\u")])).unwrap(),
            PathBuf::from("C:\\Users\\u")
        );
        // HOMEDRIVE + HOMEPATH are joined with `PathBuf::push`, which only treats `\` as a
        // separator on Windows -- so assert the join happened rather than a literal string that
        // is only correct on the platform this fallback exists for.
        let joined = home_dir_in(&env_of(&[("HOMEDRIVE", "C:"), ("HOMEPATH", "\\Users\\u")]))
            .expect("HOMEDRIVE + HOMEPATH resolve a home");
        let joined = joined.to_string_lossy();
        assert!(joined.starts_with("C:"), "{joined}");
        assert!(joined.ends_with("Users\\u"), "{joined}");
        assert!(home_dir_in(&env_of(&[])).is_none());
        // HOME wins when several are present.
        assert_eq!(
            home_dir_in(&env_of(&[
                ("HOME", "/home/u"),
                ("USERPROFILE", "C:\\Users\\u")
            ]))
            .unwrap(),
            PathBuf::from("/home/u")
        );
    }

    #[test]
    fn the_config_root_prefers_xdg_over_the_home_default() {
        assert_eq!(
            config_root_in(&env_of(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/cfg")])).unwrap(),
            PathBuf::from("/cfg")
        );
        assert_eq!(
            config_root_in(&env_of(&[("HOME", "/home/u")])).unwrap(),
            PathBuf::from("/home/u/.config")
        );
    }
}
