//! `--install-hook`, `--install-statusline`, their uninstallers, and `--uninstall`.
//!
//! The one place this tool writes into another program's file, and only when a person types the
//! command: the command is the consent, as it is for `--check-update`. The dashboard never calls
//! anything here. What it merges into Claude Code's `settings.json` is the shipped
//! `contrib/claude-code/*.json`, byte for byte the blocks the setup guide shows -- one source, so
//! the installer and the hand-merge instructions cannot drift.
//!
//! Why a command rather than the documented `jq -s '.[0] * .[1]'`: `jq`'s `*` merges objects and
//! *replaces* arrays, so a user with any other `PostToolUse` hook lost it, and three documents had
//! to warn about that. This appends to the array, removes only what it added, keeps every other
//! key in the order it found it, and preserves the file's permission bits -- `settings.json` may
//! hold an `env` block with keys, which is also why nothing here ever prints or logs any part of
//! the file but the two entries that are ours.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

use crate::utils::{home_dir_in, system_env, Env};

/// The hook block, exactly as the setup guide shows it.
pub const HOOK_TEMPLATE: &str = include_str!("../contrib/claude-code/settings.json");
/// The status line block, exactly as the setup guide shows it.
pub const STATUSLINE_TEMPLATE: &str =
    include_str!("../contrib/claude-code/statusline-settings.json");
/// The binary's name, which is what a command in `settings.json` is recognised by.
pub const PROGRAM: &str = "ai-usage-tui";
/// Both events, always: a Bash command that exits non-zero fires the second, not the first.
pub const HOOK_EVENTS: [&str; 2] = ["PostToolUse", "PostToolUseFailure"];

/// Which of the two Claude Code entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Integration {
    /// `hooks.PostToolUse[]` and `hooks.PostToolUseFailure[]`, running `--claude-code-hook`.
    Hook,
    /// `statusLine`, running `--statusline`.
    StatusLine,
}

impl Integration {
    /// The flag the entry runs this tool with.
    pub fn flag(self) -> &'static str {
        match self {
            Integration::Hook => "--claude-code-hook",
            Integration::StatusLine => "--statusline",
        }
    }

    /// The flag that installs it.
    pub fn install_flag(self) -> &'static str {
        match self {
            Integration::Hook => "--install-hook",
            Integration::StatusLine => "--install-statusline",
        }
    }

    /// How the reports name it.
    pub fn label(self) -> &'static str {
        match self {
            Integration::Hook => "Claude Code hook",
            Integration::StatusLine => "Claude Code status line",
        }
    }

    fn template(self) -> Result<Value> {
        let text = match self {
            Integration::Hook => HOOK_TEMPLATE,
            Integration::StatusLine => STATUSLINE_TEMPLATE,
        };
        serde_json::from_str(text).context("the shipped contrib/claude-code template is not JSON")
    }
}

/// How the command written into `settings.json` names the binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandForm {
    /// `ai-usage-tui`, because a file of that name is on `PATH`.
    OnPath,
    /// The absolute path of the running binary, because none is.
    AbsoluteExe,
}

/// The command an entry will run, and why it took that form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookCommand {
    pub text: String,
    pub form: CommandForm,
}

/// The command to write for `what`.
///
/// Hooks run with Claude Code's environment, so the bare name only works when a binary of that
/// name is on `PATH`; the docs used to tell the user to check `command -v` and paste an absolute
/// path by hand. This checks, and writes the absolute path of the running binary when the name
/// would not resolve. The report says which, because the two age differently: the bare name
/// follows an upgrade through any channel, the absolute path breaks when the binary moves.
pub fn command_for(what: Integration) -> Result<HookCommand> {
    command_for_in(what, &system_env, std::env::current_exe)
}

pub fn command_for_in(
    what: Integration,
    env: Env<'_>,
    current_exe: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<HookCommand> {
    let on_path = env("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .any(|dir| dir.join(PROGRAM).is_file() || dir.join(format!("{PROGRAM}.exe")).is_file())
    });
    if on_path {
        return Ok(HookCommand {
            text: format!("{PROGRAM} {}", what.flag()),
            form: CommandForm::OnPath,
        });
    }
    let exe = current_exe().context("could not determine this binary's own path")?;
    Ok(HookCommand {
        text: format!("{} {}", quote(&exe.to_string_lossy()), what.flag()),
        form: CommandForm::AbsoluteExe,
    })
}

/// Claude Code runs the command through a shell, so a path with a space in it needs quoting.
fn quote(word: &str) -> String {
    if word.chars().any(char::is_whitespace) || word.contains(['"', '\'']) {
        format!("\"{}\"", word.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        word.to_string()
    }
}

/// Where Claude Code's `settings.json` is.
///
/// The same precedence as `collector::claude_code::config_json_path`: `$CLAUDE_CONFIG_DIR`,
/// where Claude Code keeps everything when that is set; else one level above the session-log
/// root -- `~/.claude/projects` sits beside `~/.claude/settings.json` -- so `--claude-dir` names
/// it too, which is what keeps tests away from the developer's real file; else `~/.claude`.
pub fn settings_path(claude_dir: Option<&Path>) -> Option<PathBuf> {
    settings_path_in(claude_dir, &system_env)
}

pub fn settings_path_in(claude_dir: Option<&Path>, env: Env<'_>) -> Option<PathBuf> {
    let set = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(config) = set("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(config).join("settings.json"));
    }
    let root = claude_dir
        .map(Path::to_path_buf)
        .or_else(|| set("CLAUDE_PROJECTS_DIR").map(PathBuf::from));
    if let Some(root) = root {
        return Some(root.parent()?.join("settings.json"));
    }
    Some(home_dir_in(env)?.join(".claude").join("settings.json"))
}

/// Whether a `command` string in `settings.json` runs this tool with `what`'s flag.
///
/// Recognised by shape, not by string: the program is the first shell word, and it is ours when
/// its file name is `ai-usage-tui` (or `.exe`) in whichever form the installer or a hand edit
/// wrote it -- bare, absolute, quoted, Windows -- or when it is this very binary under another
/// name. The flag must be one of the remaining words, so `ai-usage-tui --statusline` inside a
/// hook list is not the hook, and `ai-usage-tui-else --claude-code-hook` is nobody's.
pub fn is_ours(command: &str, what: Integration) -> bool {
    is_ours_with(command, what, std::env::current_exe().ok().as_deref())
}

pub fn is_ours_with(command: &str, what: Integration, this_exe: Option<&Path>) -> bool {
    let words = shell_words(command);
    let Some(program) = words.first() else {
        return false;
    };
    let file_name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let file_name = file_name.strip_suffix(".exe").unwrap_or(file_name);
    let ours = file_name == PROGRAM || this_exe.is_some_and(|exe| Path::new(program) == exe);
    ours && words[1..].iter().any(|word| word == what.flag())
}

/// Split as `sh` would, far enough for a command line: whitespace separates, single and double
/// quotes group, and a backslash outside single quotes escapes a following space, quote or
/// backslash. Only those: a backslash before anything else is kept, so a Windows path
/// (`C:\Users\...\ai-usage-tui.exe`) reads as a path and not as its letters.
fn shell_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\')
                if chars.peek().is_some_and(|next| {
                    next.is_whitespace() || ['"', '\'', '\\'].contains(next)
                }) =>
            {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
                in_word = true;
            }
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(current);
    }
    words
}

/// What a settings document holds of ours: the command found, per place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Installed {
    /// The hook events that run this tool, with the command each runs. Two is installed; one is
    /// half a hook, and `--doctor` says so.
    pub hook: Vec<(&'static str, String)>,
    /// The status line's command, when it is ours.
    pub statusline: Option<String>,
}

/// The one detector: `--doctor`, the installers and the uninstallers all ask this, so what
/// `--doctor` calls installed is exactly what `--uninstall-hook` will remove.
pub fn detect(settings: &Map<String, Value>) -> Installed {
    let hook = settings
        .get("hooks")
        .and_then(Value::as_object)
        .map(|hooks| {
            HOOK_EVENTS
                .iter()
                .filter_map(|event| {
                    let list = hooks.get(*event)?.as_array()?;
                    let command = list
                        .iter()
                        .find_map(|entry| ours_in_entry(entry, Integration::Hook))?;
                    Some((*event, command))
                })
                .collect()
        })
        .unwrap_or_default();
    let statusline = settings
        .get("statusLine")
        .and_then(|line| line.get("command"))
        .and_then(Value::as_str)
        .filter(|command| is_ours(command, Integration::StatusLine))
        .map(str::to_string);
    Installed { hook, statusline }
}

/// The command of ours inside one `{matcher, hooks: [...]}` entry, if any.
fn ours_in_entry(entry: &Value, what: Integration) -> Option<String> {
    entry.get("hooks")?.as_array()?.iter().find_map(|hook| {
        hook.get("command")
            .and_then(Value::as_str)
            .filter(|command| is_ours(command, what))
            .map(str::to_string)
    })
}

/// A settings file as `--doctor` reports it. Never an error: an unreadable file is a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsState {
    Missing,
    Parsed(Installed),
    Unreadable(String),
}

pub fn detect_at(path: &Path) -> SettingsState {
    match read_settings(path) {
        Ok(None) => SettingsState::Missing,
        Ok(Some(settings)) => SettingsState::Parsed(detect(&settings)),
        Err(error) => SettingsState::Unreadable(format!("{error:#}")),
    }
}

/// What an install or uninstall did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Written.
    Installed,
    /// Every place already ran this command; nothing written.
    AlreadyInstalled(String),
    /// Written.
    Removed,
    /// There was nothing of ours to remove; nothing written.
    NotInstalled,
    /// The file does not exist; nothing written.
    NoFile,
    /// The status line belongs to something else, named here, and was left alone.
    NotOurs(String),
}

/// Merge `what` into the settings at `path`, writing only if something changed.
pub fn install(path: &Path, what: Integration, command: &str) -> Result<Outcome> {
    let mut settings = read_settings(path)?.unwrap_or_default();
    let outcome = match what {
        Integration::Hook => {
            if install_hook(&mut settings, command)? {
                Outcome::Installed
            } else {
                let found = detect(&settings)
                    .hook
                    .into_iter()
                    .next()
                    .map(|(_, command)| command)
                    .unwrap_or_else(|| command.to_string());
                Outcome::AlreadyInstalled(found)
            }
        }
        Integration::StatusLine => install_statusline(&mut settings, command)?,
    };
    if outcome == Outcome::Installed {
        write_settings(path, &settings)?;
    }
    Ok(outcome)
}

/// Remove `what` from the settings at `path`, writing only if something was removed.
pub fn uninstall(path: &Path, what: Integration) -> Result<Outcome> {
    let Some(mut settings) = read_settings(path)? else {
        return Ok(Outcome::NoFile);
    };
    let outcome = match what {
        Integration::Hook => {
            if uninstall_hook(&mut settings) {
                Outcome::Removed
            } else {
                Outcome::NotInstalled
            }
        }
        Integration::StatusLine => uninstall_statusline(&mut settings),
    };
    if outcome == Outcome::Removed {
        write_settings(path, &settings)?;
    }
    Ok(outcome)
}

/// Append our entry to each event's list that lacks one. `true` if any was appended.
fn install_hook(settings: &mut Map<String, Value>, command: &str) -> Result<bool> {
    let template = Integration::Hook.template()?;
    let hooks = settings
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or_else(|| {
        anyhow!("`hooks` in the settings is not a JSON object, so nothing was written")
    })?;
    let mut changed = false;
    for event in HOOK_EVENTS {
        let list = hooks
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()));
        let list = list.as_array_mut().ok_or_else(|| {
            anyhow!("`hooks.{event}` in the settings is not a JSON array, so nothing was written")
        })?;
        if list
            .iter()
            .any(|entry| ours_in_entry(entry, Integration::Hook).is_some())
        {
            continue;
        }
        let mut entry = template
            .pointer(&format!("/hooks/{event}/0"))
            .cloned()
            .ok_or_else(|| anyhow!("the shipped hook template has no {event} entry"))?;
        match entry.pointer_mut("/hooks/0/command") {
            Some(slot) => *slot = Value::String(command.to_string()),
            None => bail!("the shipped hook template's {event} entry has no command"),
        }
        list.push(entry);
        changed = true;
    }
    Ok(changed)
}

/// Remove our hook objects, and only what their removal emptied. `true` if any was removed.
fn uninstall_hook(settings: &mut Map<String, Value>) -> bool {
    let Some(Value::Object(hooks)) = settings.get_mut("hooks") else {
        return false;
    };
    let mut removed = false;
    for event in HOOK_EVENTS {
        let Some(Value::Array(list)) = hooks.get_mut(event) else {
            continue;
        };
        let mut emptied = Vec::new();
        for (index, entry) in list.iter_mut().enumerate() {
            let Some(Value::Array(inner)) = entry.get_mut("hooks") else {
                continue;
            };
            let before = inner.len();
            inner.retain(|hook| {
                !hook
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| is_ours(command, Integration::Hook))
            });
            if inner.len() != before {
                removed = true;
                if inner.is_empty() {
                    emptied.push(index);
                }
            }
        }
        for index in emptied.into_iter().rev() {
            list.remove(index);
        }
        // `shift_remove`, not `remove`: with `preserve_order` the latter swaps the last key into
        // the hole, and the user's file came back with its keys shuffled.
        if removed && list.is_empty() {
            hooks.shift_remove(event);
        }
    }
    if removed && hooks.is_empty() {
        settings.shift_remove("hooks");
    }
    removed
}

/// Set `statusLine` when there is none; refuse to replace another program's.
fn install_statusline(settings: &mut Map<String, Value>, command: &str) -> Result<Outcome> {
    if let Some(existing) = settings.get("statusLine") {
        let theirs = existing.get("command").and_then(Value::as_str);
        return match theirs {
            Some(current) if is_ours(current, Integration::StatusLine) => {
                Ok(Outcome::AlreadyInstalled(current.to_string()))
            }
            _ => bail!(
                "Claude Code already has a status line that is not this tool's: {}. \
                 --install-statusline would replace it with `{command}`. Remove it first, or \
                 have it also run `{PROGRAM} --statusline` (see contrib/claude-code/README.md). \
                 Nothing was written.",
                describe_statusline(existing)
            ),
        };
    }
    let mut line = Integration::StatusLine
        .template()?
        .get("statusLine")
        .cloned()
        .ok_or_else(|| anyhow!("the shipped status line template has no statusLine"))?;
    match line.get_mut("command") {
        Some(slot) => *slot = Value::String(command.to_string()),
        None => bail!("the shipped status line template has no command"),
    }
    settings.insert("statusLine".to_string(), line);
    Ok(Outcome::Installed)
}

/// Remove `statusLine` when it is ours; leave and name it when it is not.
fn uninstall_statusline(settings: &mut Map<String, Value>) -> Outcome {
    let Some(existing) = settings.get("statusLine") else {
        return Outcome::NotInstalled;
    };
    let theirs = existing.get("command").and_then(Value::as_str);
    match theirs {
        Some(current) if is_ours(current, Integration::StatusLine) => {
            settings.shift_remove("statusLine");
            Outcome::Removed
        }
        _ => Outcome::NotOurs(describe_statusline(existing)),
    }
}

/// The other program's status line, for a message: its command when it has one, else the block.
fn describe_statusline(line: &Value) -> String {
    match line.get("command").and_then(Value::as_str) {
        Some(command) => format!("`{command}`"),
        None => line.to_string(),
    }
}

/// The settings object, `None` when the file does not exist.
///
/// A file that is not a JSON object is an error that names the path and serde's position, and
/// never a byte of the file: the fix is the user's, by hand. Whitespace alone reads as `{}`,
/// which is what an editor's empty save means.
fn read_settings(path: &Path) -> Result<Option<Map<String, Value>>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()))
        }
    };
    if text.trim().is_empty() {
        return Ok(Some(Map::new()));
    }
    let value: Value = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} is not JSON this tool can read; fix it by hand, nothing was written",
            path.display()
        )
    })?;
    match value {
        Value::Object(settings) => Ok(Some(settings)),
        other => bail!(
            "{} holds a JSON {} where Claude Code's settings object was expected; nothing was written",
            path.display(),
            json_kind(&other)
        ),
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Write the settings back the way Claude Code writes them -- two-space indent, trailing
/// newline -- keeping the file's permission bits, or owner-only for a file that did not exist.
fn write_settings(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt as _;
        Some(
            std::fs::metadata(path)
                .map(|meta| meta.permissions().mode() & 0o777)
                .unwrap_or(0o600),
        )
    };
    #[cfg(not(unix))]
    let mode = None;
    let mut text = serde_json::to_string_pretty(settings)?;
    text.push('\n');
    crate::helpers::write_atomic_with_mode(path, text.as_bytes(), mode)
        .with_context(|| format!("could not write {}", path.display()))
}

/// The files this tool keeps for itself, and the two that are the user's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnFiles {
    /// Rebuilt on demand; `--uninstall` removes them. Listed in `docs/stability.md` under
    /// "Files the tool keeps for itself", and a test holds the two lists together.
    pub caches: Vec<PathBuf>,
    /// The journal. Never removed by this tool.
    pub journal: PathBuf,
    /// The config file, when a location for it could be resolved. Never removed by this tool.
    pub config: Option<PathBuf>,
}

pub fn own_files(journal: PathBuf, config: Option<PathBuf>) -> OwnFiles {
    let caches = [
        crate::collector::pricing_refresh::pricing_cache_path(),
        crate::collector::zen::zen_cache_path(),
        crate::update::check_cache_path(),
        crate::statusline::cache_path(),
        crate::logging::default_log_path(),
        crate::logging::default_log_backup_path(),
    ]
    .into_iter()
    .flatten()
    .collect();
    OwnFiles {
        caches,
        journal,
        config,
    }
}

/// Remove each cache, reporting per file whether it was there. An error is returned, not
/// swallowed: a cache that could not be removed is the one thing `--uninstall` has to say.
pub fn remove_caches(caches: &[PathBuf]) -> Vec<(PathBuf, std::io::Result<bool>)> {
    caches
        .iter()
        .map(|path| {
            let result = match std::fs::remove_file(path) {
                Ok(()) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error),
            };
            (path.clone(), result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ai-usage-tui-install-{name}-{}-{}",
            std::process::id(),
            crate::utils::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parse(text: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(text)
            .unwrap()
            .as_object()
            .unwrap()
            .clone()
    }

    const BARE_HOOK: &str = "ai-usage-tui --claude-code-hook";
    const BARE_STATUSLINE: &str = "ai-usage-tui --statusline";

    /// Bug: a hand-built JSON block that drifts from the shipped contrib file.
    #[test]
    fn installing_into_empty_settings_equals_the_shipped_file() {
        let dir = scratch("empty");
        let path = dir.join("settings.json");
        assert_eq!(
            install(&path, Integration::Hook, BARE_HOOK).unwrap(),
            Outcome::Installed
        );
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let shipped: Value = serde_json::from_str(HOOK_TEMPLATE).unwrap();
        assert_eq!(written, shipped);

        let path = dir.join("statusline.json");
        assert_eq!(
            install(&path, Integration::StatusLine, BARE_STATUSLINE).unwrap(),
            Outcome::Installed
        );
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let shipped: Value = serde_json::from_str(STATUSLINE_TEMPLATE).unwrap();
        assert_eq!(written, shipped);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Bug: the array replaced, as `jq -s '.[0] * .[1]'` does -- the user's hook gone.
    #[test]
    fn install_appends_to_an_existing_array() {
        let mut settings = parse(
            r#"{"hooks":{"PostToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"prettier --write"}]}]}}"#,
        );
        assert!(install_hook(&mut settings, BARE_HOOK).unwrap());
        let list = settings["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{list:?}");
        assert_eq!(list[0]["matcher"], "Edit", "the user's entry stays first");
        assert_eq!(list[1]["hooks"][0]["command"], BARE_HOOK);
        assert_eq!(
            settings["hooks"]["PostToolUseFailure"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "the failure event is added even when only the other existed"
        );
    }

    /// Bug: a second run adds a second copy.
    #[test]
    fn installing_twice_is_one_entry() {
        let dir = scratch("twice");
        let path = dir.join("settings.json");
        install(&path, Integration::Hook, BARE_HOOK).unwrap();
        let first = std::fs::read(&path).unwrap();
        assert_eq!(
            install(&path, Integration::Hook, BARE_HOOK).unwrap(),
            Outcome::AlreadyInstalled(BARE_HOOK.to_string())
        );
        assert_eq!(std::fs::read(&path).unwrap(), first, "bytes unchanged");
        install(&path, Integration::StatusLine, BARE_STATUSLINE).unwrap();
        let second = std::fs::read(&path).unwrap();
        assert_eq!(
            install(&path, Integration::StatusLine, BARE_STATUSLINE).unwrap(),
            Outcome::AlreadyInstalled(BARE_STATUSLINE.to_string())
        );
        assert_eq!(std::fs::read(&path).unwrap(), second);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Bug: recognising only the exact string the installer writes, so a hand-pasted absolute
    /// path is installed a second time.
    #[test]
    fn the_absolute_and_exe_forms_count_as_installed() {
        for command in [
            "/usr/local/bin/ai-usage-tui --claude-code-hook",
            r"C:\Users\x\scoop\shims\ai-usage-tui.exe --claude-code-hook",
            "ai-usage-tui --config /c/config.toml --claude-code-hook",
            "\"/p q/ai-usage-tui\" --claude-code-hook",
            "'/p q/ai-usage-tui' --claude-code-hook",
        ] {
            assert!(
                is_ours_with(command, Integration::Hook, None),
                "{command} is ours"
            );
            let mut settings = parse(&format!(
                r#"{{"hooks":{{"PostToolUse":[{{"matcher":"Bash","hooks":[{{"type":"command","command":{}}}]}}],"PostToolUseFailure":[{{"matcher":"Bash","hooks":[{{"type":"command","command":{}}}]}}]}}}}"#,
                Value::String(command.to_string()),
                Value::String(command.to_string())
            ));
            assert!(
                !install_hook(&mut settings, BARE_HOOK).unwrap(),
                "{command} was installed again"
            );
        }
        // A binary under another name is still ours when it is this very binary.
        let exe = PathBuf::from("/opt/tools/aiu");
        assert!(is_ours_with(
            "/opt/tools/aiu --claude-code-hook",
            Integration::Hook,
            Some(&exe)
        ));
        assert!(!is_ours_with(
            "/opt/tools/aiu --claude-code-hook",
            Integration::Hook,
            None
        ));
    }

    /// Bug: matching the program alone, or by substring.
    #[test]
    fn same_program_other_flag_is_not_ours() {
        assert!(!is_ours_with(BARE_STATUSLINE, Integration::Hook, None));
        assert!(!is_ours_with(BARE_HOOK, Integration::StatusLine, None));
        assert!(!is_ours_with(
            "ai-usage-tui-else --claude-code-hook",
            Integration::Hook,
            None
        ));
        assert!(!is_ours_with(
            "my-ai-usage-tui --claude-code-hook",
            Integration::Hook,
            None
        ));
        assert!(!is_ours_with(
            "echo ai-usage-tui --claude-code-hook",
            Integration::Hook,
            None
        ));
        assert!(!is_ours_with("", Integration::Hook, None));
        let settings = parse(
            r#"{"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"ai-usage-tui --statusline"}]}]}}"#,
        );
        assert!(detect(&settings).hook.is_empty());
    }

    /// Bug: dropping the whole entry or the whole array along with our hook.
    #[test]
    fn uninstall_keeps_the_users_other_hooks() {
        let mut settings = parse(
            r#"{"hooks":{
                "PostToolUse":[
                    {"matcher":"Bash","hooks":[
                        {"type":"command","command":"ai-usage-tui --claude-code-hook","timeout":30},
                        {"type":"command","command":"notify-send done"}]},
                    {"matcher":"Edit","hooks":[{"type":"command","command":"prettier --write"}]}],
                "PostToolUseFailure":[
                    {"matcher":"Bash","hooks":[{"type":"command","command":"/usr/bin/ai-usage-tui --claude-code-hook"}]}],
                "Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#,
        );
        assert!(uninstall_hook(&mut settings));
        let list = settings["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["hooks"].as_array().unwrap().len(), 1);
        assert_eq!(list[0]["hooks"][0]["command"], "notify-send done");
        assert_eq!(list[1]["matcher"], "Edit");
        assert!(
            settings["hooks"].get("PostToolUseFailure").is_none(),
            "an event list we emptied goes"
        );
        assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(detect(&settings).hook.is_empty());
        assert!(
            !uninstall_hook(&mut settings),
            "a second removal finds nothing"
        );
    }

    /// Bug: `"hooks": {}` left behind, or the other keys reordered or touched.
    #[test]
    fn uninstall_prunes_only_what_it_emptied() {
        let mut settings = parse(
            r#"{"zeta":1,"env":{"ANTHROPIC_API_KEY":"sk-secret"},"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"ai-usage-tui --claude-code-hook","timeout":30}]}],"PostToolUseFailure":[{"matcher":"Bash","hooks":[{"type":"command","command":"ai-usage-tui --claude-code-hook","timeout":30}]}]},"permissions":{"allow":["Bash"]},"alpha":2}"#,
        );
        assert!(uninstall_hook(&mut settings));
        assert_eq!(
            settings.keys().collect::<Vec<_>>(),
            ["zeta", "env", "permissions", "alpha"]
        );
        assert_eq!(settings["env"]["ANTHROPIC_API_KEY"], "sk-secret");

        // A pre-existing empty `hooks` object that we did not empty is not ours to prune.
        let mut settings = parse(r#"{"hooks":{}}"#);
        assert!(!uninstall_hook(&mut settings));
        assert!(settings.contains_key("hooks"));
    }

    /// Bug: the file written back alphabetised -- what `serde_json` does without
    /// `preserve_order`, on a file the user edits by hand.
    #[test]
    fn unknown_keys_keep_their_order() {
        let dir = scratch("order");
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"zeta":{"b":1,"a":2},"model":"opus","alpha":[3,1,2]}"#,
        )
        .unwrap();
        install(&path, Integration::Hook, BARE_HOOK).unwrap();
        install(&path, Integration::StatusLine, BARE_STATUSLINE).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let written = parse(&text);
        assert_eq!(
            written.keys().collect::<Vec<_>>(),
            ["zeta", "model", "alpha", "hooks", "statusLine"]
        );
        assert_eq!(
            written["zeta"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(text.ends_with("}\n"), "trailing newline");
        assert!(text.contains("\n  \"hooks\": {"), "two-space indent");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Bug: a file that is not a settings object silently overwritten.
    #[test]
    fn a_settings_file_that_is_not_an_object_is_refused() {
        let dir = scratch("refused");
        for (name, body) in [
            ("array", "[]"),
            ("string", "\"sk-secret-value\""),
            ("broken", "{ \"env\": { \"KEY\": \"sk-secret-value\" "),
        ] {
            let path = dir.join(format!("{name}.json"));
            std::fs::write(&path, body).unwrap();
            let error = install(&path, Integration::Hook, BARE_HOOK)
                .unwrap_err()
                .to_string();
            assert!(error.contains(&path.display().to_string()), "{error}");
            assert!(
                !error.contains("sk-secret-value"),
                "the file's contents are never echoed: {error}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), body, "unchanged");
            let error = uninstall(&path, Integration::Hook).unwrap_err().to_string();
            assert!(!error.contains("sk-secret-value"), "{error}");
            assert!(matches!(detect_at(&path), SettingsState::Unreadable(_)));
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Bug: `hooks` that is not an object clobbered with one.
    #[test]
    fn hooks_that_is_not_an_object_is_refused() {
        let mut settings = parse(r#"{"hooks":[]}"#);
        assert!(install_hook(&mut settings, BARE_HOOK).is_err());
        let mut settings = parse(r#"{"hooks":{"PostToolUse":{"matcher":"Bash"}}}"#);
        assert!(install_hook(&mut settings, BARE_HOOK).is_err());
        assert!(!uninstall_hook(&mut settings), "and removal leaves it too");
        assert_eq!(settings["hooks"]["PostToolUse"]["matcher"], "Bash");
    }

    /// Bug: a `0600` file rewritten at the umask.
    #[cfg(unix)]
    #[test]
    fn permissions_are_preserved() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("mode");
        for mode in [0o600, 0o644, 0o640] {
            let path = dir.join(format!("{mode:o}.json"));
            std::fs::write(&path, "{}").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            install(&path, Integration::Hook, BARE_HOOK).unwrap();
            let after = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(after, mode, "{mode:o} became {after:o}");
        }
        let path = dir.join("new").join("settings.json");
        install(&path, Integration::Hook, BARE_HOOK).unwrap();
        let after = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(after, 0o600, "a file that did not exist is owner-only");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Bug: another program's status line silently replaced, or removed.
    #[test]
    fn another_tools_status_line_is_refused_and_named() {
        let dir = scratch("statusline");
        let path = dir.join("settings.json");
        let body = r#"{"statusLine":{"type":"command","command":"starship prompt"}}"#;
        std::fs::write(&path, body).unwrap();
        let error = install(&path, Integration::StatusLine, BARE_STATUSLINE)
            .unwrap_err()
            .to_string();
        assert!(error.contains("starship prompt"), "{error}");
        assert!(error.contains(BARE_STATUSLINE), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
        assert_eq!(
            uninstall(&path, Integration::StatusLine).unwrap(),
            Outcome::NotOurs("`starship prompt`".to_string())
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
        match detect_at(&path) {
            SettingsState::Parsed(found) => assert_eq!(found.statusline, None),
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_file_and_directory_are_created_and_a_missing_file_uninstalls_to_nothing() {
        let dir = scratch("missing");
        let path = dir.join("deeper").join(".claude").join("settings.json");
        assert_eq!(
            uninstall(&path, Integration::Hook).unwrap(),
            Outcome::NoFile
        );
        assert!(!path.exists(), "an uninstall creates nothing");
        assert_eq!(detect_at(&path), SettingsState::Missing);
        install(&path, Integration::StatusLine, BARE_STATUSLINE).unwrap();
        assert!(path.is_file());
        assert_eq!(
            uninstall(&path, Integration::StatusLine).unwrap(),
            Outcome::Removed
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}\n");
        assert_eq!(
            uninstall(&path, Integration::StatusLine).unwrap(),
            Outcome::NotInstalled
        );
        // Whitespace alone is an empty object, not a parse error.
        std::fs::write(&path, "\n  \n").unwrap();
        assert_eq!(
            install(&path, Integration::Hook, BARE_HOOK).unwrap(),
            Outcome::Installed
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Bug: a fixture-only test resolving the developer's real `~/.claude/settings.json`.
    #[test]
    fn settings_path_follows_claude_dir() {
        let none = |_: &str| None::<OsString>;
        assert_eq!(
            settings_path_in(Some(Path::new("/x/.claude/projects")), &none),
            Some(PathBuf::from("/x/.claude/settings.json"))
        );
        let config_dir = |name: &str| match name {
            "CLAUDE_CONFIG_DIR" => Some(OsString::from("/c")),
            "HOME" => Some(OsString::from("/h")),
            _ => None,
        };
        assert_eq!(
            settings_path_in(Some(Path::new("/x/.claude/projects")), &config_dir),
            Some(PathBuf::from("/c/settings.json")),
            "CLAUDE_CONFIG_DIR wins, as it does for .claude.json"
        );
        let projects_dir = |name: &str| match name {
            "CLAUDE_PROJECTS_DIR" => Some(OsString::from("/p/projects")),
            "CLAUDE_CONFIG_DIR" => Some(OsString::new()),
            "HOME" => Some(OsString::from("/h")),
            _ => None,
        };
        assert_eq!(
            settings_path_in(None, &projects_dir),
            Some(PathBuf::from("/p/settings.json")),
            "an empty CLAUDE_CONFIG_DIR is unset"
        );
        let home = |name: &str| (name == "HOME").then(|| OsString::from("/h"));
        assert_eq!(
            settings_path_in(None, &home),
            Some(PathBuf::from("/h/.claude/settings.json"))
        );
        assert_eq!(settings_path_in(None, &none), None);
    }

    /// Bug: writing a name Claude Code's shell cannot resolve.
    #[test]
    fn the_command_is_bare_on_path_and_absolute_otherwise() {
        let dir = scratch("path");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = || Ok(PathBuf::from("/opt/some dir/ai-usage-tui"));

        let empty_path = |name: &str| (name == "PATH").then(|| OsString::from("/nonexistent"));
        let command = command_for_in(Integration::Hook, &empty_path, exe).unwrap();
        assert_eq!(command.form, CommandForm::AbsoluteExe);
        assert_eq!(
            command.text,
            "\"/opt/some dir/ai-usage-tui\" --claude-code-hook"
        );
        assert!(is_ours_with(&command.text, Integration::Hook, None));

        std::fs::write(bin.join(PROGRAM), "").unwrap();
        let with_bin = |name: &str| (name == "PATH").then(|| bin.clone().into_os_string());
        let command = command_for_in(Integration::StatusLine, &with_bin, exe).unwrap();
        assert_eq!(command.form, CommandForm::OnPath);
        assert_eq!(command.text, BARE_STATUSLINE);

        let no_path = |_: &str| None::<OsString>;
        let plain = || Ok(PathBuf::from("/usr/bin/ai-usage-tui"));
        let command = command_for_in(Integration::Hook, &no_path, plain).unwrap();
        assert_eq!(command.text, "/usr/bin/ai-usage-tui --claude-code-hook");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shell_words_splits_like_sh() {
        assert_eq!(shell_words("a  b\tc"), ["a", "b", "c"]);
        assert_eq!(shell_words("\"a b\" c"), ["a b", "c"]);
        assert_eq!(shell_words("'a b' c"), ["a b", "c"]);
        assert_eq!(shell_words("a\\ b c"), ["a b", "c"]);
        assert_eq!(shell_words("\"a\\\"b\""), ["a\"b"]);
        assert_eq!(shell_words("'a\\b'"), ["a\\b"]);
        assert_eq!(
            shell_words(r"C:\Users\x\ai-usage-tui.exe --statusline"),
            [r"C:\Users\x\ai-usage-tui.exe", "--statusline"]
        );
        assert_eq!(shell_words(r"a\\b"), [r"a\b"]);
        assert_eq!(shell_words("  "), Vec::<String>::new());
    }

    #[test]
    fn detect_reports_half_a_hook() {
        let settings = parse(
            r#"{"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"ai-usage-tui --claude-code-hook"}]}]}}"#,
        );
        let found = detect(&settings);
        assert_eq!(found.hook, [("PostToolUse", BARE_HOOK.to_string())]);
        assert_eq!(found.statusline, None);
    }

    #[test]
    fn remove_caches_tells_present_from_absent_from_failed() {
        let dir = scratch("caches");
        let present = dir.join("present.json");
        std::fs::write(&present, "{}").unwrap();
        let absent = dir.join("absent.json");
        let blocked = dir.join("dir");
        std::fs::create_dir_all(&blocked).unwrap();
        let results = remove_caches(&[present.clone(), absent.clone(), blocked.clone()]);
        assert!(matches!(results[0], (ref p, Ok(true)) if *p == present));
        assert!(matches!(results[1], (ref p, Ok(false)) if *p == absent));
        assert!(matches!(results[2], (ref p, Err(_)) if *p == blocked));
        assert!(!present.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
