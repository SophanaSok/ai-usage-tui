//! What a read went around rather than through.
//!
//! Every tailing collector used to treat an unreadable file as `Err(_) => continue` and a line
//! that was not JSON as "no usage here", with no count anywhere. Both are the right call for
//! resilience -- one bad transcript must not sink the source -- and both left the totals short
//! with nothing on screen saying so, which is convention 8's silent failure. This is the one
//! place those reads now record what they skipped, so `Collector::warning` can put it on the
//! status line and the one-shot status can carry it into `--once`, `--json` and `--doctor`.

use std::path::{Path, PathBuf};

/// Carried inside a collector's cursor, so it lives exactly as long as what it describes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Skipped {
    /// Files that could not be opened or read on the latest pass, with the first error. Their
    /// offsets did not move, so each is retried on the next poll: this is a current state,
    /// cleared at the start of every pass, and a file that becomes readable drops out of it.
    unreadable: Vec<(PathBuf, String)>,
    /// Complete records that were not valid JSON. The cursor has moved past them, so their usage
    /// is gone for this process: this only grows.
    malformed: u64,
    /// Records kept although a token count every record of the source carries was absent. Their
    /// other counts are in the totals; they are never priced. Only grows.
    incomplete: u64,
    /// Records kept although they carry no usable timestamp. They are in `--all` and in no other
    /// range, no day and no budget period -- which, uncounted, reads as less usage. Only grows.
    undated: u64,
}

impl Skipped {
    /// Forget the previous pass's unreadable files; they are about to be tried again.
    pub fn begin_pass(&mut self) {
        self.unreadable.clear();
    }

    pub fn unreadable(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.unreadable
            .push((path.to_path_buf(), error.to_string()));
    }

    pub fn malformed(&mut self) {
        self.malformed = self.malformed.saturating_add(1);
    }

    /// Count what a kept row is missing. Call once per row, where the cursor guarantees once.
    pub fn note(&mut self, usage: &crate::model::Usage) {
        if usage.incomplete {
            self.incomplete = self.incomplete.saturating_add(1);
        }
        if usage.created <= 0 {
            self.undated = self.undated.saturating_add(1);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.unreadable.is_empty()
            && self.malformed == 0
            && self.incomplete == 0
            && self.undated == 0
    }

    /// The short form, for the dashboard's status line; `None` when nothing was skipped.
    pub fn warning(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.unreadable.is_empty() {
            parts.push(format!("{} file(s) unreadable", self.unreadable.len()));
        }
        if self.malformed > 0 {
            parts.push(format!("{} malformed record(s) skipped", self.malformed));
        }
        if self.incomplete > 0 {
            parts.push(format!(
                "{} record(s) missing a token count, left unpriced",
                self.incomplete
            ));
        }
        if self.undated > 0 {
            parts.push(format!(
                "{} record(s) with no timestamp, in no range but --all",
                self.undated
            ));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    /// The long form, naming the first unreadable file and why -- for the one-shot status and
    /// the log, where there is room for a path.
    pub fn detail(&self) -> Option<String> {
        let warning = self.warning()?;
        Some(match self.unreadable.first() {
            Some((path, error)) => format!("{warning}; first: {}: {error}", path.display()),
            None => warning,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_skipped_says_nothing() {
        let skipped = Skipped::default();
        assert!(skipped.is_empty());
        assert_eq!(skipped.warning(), None);
        assert_eq!(skipped.detail(), None);
    }

    #[test]
    fn unreadable_files_are_a_current_state_and_malformed_records_accumulate() {
        let mut skipped = Skipped::default();
        skipped.begin_pass();
        skipped.unreadable(Path::new("/logs/a.jsonl"), "permission denied");
        skipped.malformed();
        assert_eq!(
            skipped.detail().as_deref(),
            Some(
                "1 file(s) unreadable, 1 malformed record(s) skipped; first: /logs/a.jsonl: permission denied"
            )
        );

        // Next pass: the file became readable, and the malformed record stays lost.
        skipped.begin_pass();
        skipped.malformed();
        assert_eq!(
            skipped.warning().as_deref(),
            Some("2 malformed record(s) skipped")
        );
    }
}
