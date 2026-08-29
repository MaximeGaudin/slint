//! Applying the fixes that are computed rather than written.
//!
//! Only ever computed ones: a model never edits a file here. A fix is a byte range and a
//! replacement, so applying several to one file is a sort and a splice — and two fixes that overlap
//! mean one of them was resolved against text the other already changed, so the second is left for
//! the next pass rather than applied against a moved target. Files are replaced by rename rather
//! than rewritten in place, so a crash mid-fix never leaves half of each.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::diagnostics::{Fix, Report};

/// What `--fix` did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    pub files: usize,
    pub fixes: usize,
    /// Fixes that overlapped one already applied, and are left for the next pass.
    pub deferred: usize,
    /// Files whose fixes were not applied, with why. A run that could not finish a file still
    /// reports everything else it did, so the reader is never left guessing which files changed.
    pub failed: Vec<FileFailure>,
}

/// One file `--fix` could not finish, and what stopped it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFailure {
    /// The path the fixes name, as the report spells it.
    pub file: String,
    /// The error, as a sentence a reader can act on.
    pub reason: String,
}

/// Applies every fix in a report, file by file.
///
/// A file that cannot be fixed is recorded and left behind rather than aborting the run: the files
/// fixed before it are already on disk, so an early return would trade one known outcome for a
/// mystery about the rest.
pub fn apply(report: &Report) -> Applied {
    let mut by_file: BTreeMap<&str, Vec<&Fix>> = BTreeMap::new();

    for skill in &report.skills {
        for message in &skill.messages {
            if let Some(fix) = &message.fix {
                by_file.entry(message.file.as_str()).or_default().push(fix);
            }
        }
    }

    let mut applied = Applied::default();

    for (file, fixes) in by_file {
        let outcome = apply_to_file(Path::new(file), &fixes);

        match outcome {
            Ok(outcome) => {
                if outcome.fixes > 0 {
                    applied.files += 1;
                }

                applied.fixes += outcome.fixes;
                applied.deferred += outcome.deferred;
            }
            Err(failure) => applied.failed.push(FileFailure {
                file: file.to_string(),
                reason: format!("{failure:#}"),
            }),
        }
    }

    applied
}

fn apply_to_file(path: &Path, fixes: &[&Fix]) -> Result<Applied> {
    let permission_changes = fixes
        .iter()
        .filter(|fix| fix.start == 0 && fix.end == 0 && fix.replacement.is_empty())
        .count();

    let mut applied = Applied::default();

    if permission_changes > 0 {
        make_executable(path)?;
        applied.fixes += permission_changes;
    }

    let edits: Vec<&&Fix> = fixes
        .iter()
        .filter(|fix| !(fix.start == 0 && fix.end == 0 && fix.replacement.is_empty()))
        .collect();

    if edits.is_empty() {
        return Ok(applied);
    }

    let original =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (patched, count, deferred) = patch(&original, &edits);

    if count > 0 {
        write_atomically(path, &patched).with_context(|| format!("writing {}", path.display()))?;
    }

    applied.fixes += count;
    applied.deferred += deferred;

    Ok(applied)
}

/// Splices fixes into text, last one first so earlier offsets stay valid.
pub fn patch(text: &str, fixes: &[&&Fix]) -> (String, usize, usize) {
    let mut ordered: Vec<&&&Fix> = fixes.iter().collect();
    // Descending, so splicing one in never moves the offsets of the ones still to come.
    ordered.sort_by_key(|fix| std::cmp::Reverse(fix.start));

    let mut patched = text.to_string();
    let mut applied = 0;
    let mut deferred = 0;
    let mut lowest_applied = usize::MAX;

    for fix in ordered {
        if fix.end > patched.len() || fix.start > fix.end {
            deferred += 1;
            continue;
        }

        // Overlapping the range of one already applied means this was computed against text that
        // has since moved. Leaving it for the next pass is how a fixer stays idempotent.
        if fix.end > lowest_applied {
            deferred += 1;
            continue;
        }

        if !patched.is_char_boundary(fix.start) || !patched.is_char_boundary(fix.end) {
            deferred += 1;
            continue;
        }

        patched.replace_range(fix.start..fix.end, &fix.replacement);
        lowest_applied = fix.start;
        applied += 1;
    }

    (patched, applied, deferred)
}

/// Replaces a file by rename rather than truncating it in place.
///
/// The new content is written to a temporary file in the same directory, flushed, and renamed over
/// the original. rename(2) is atomic, so a crash or a full disk mid-write leaves either the old
/// content or the new one on disk — never a half-written file and no way back to the original.
fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));

    let temporary = tempfile::Builder::new()
        .prefix(".slint-fix-")
        .tempfile_in(directory)
        .with_context(|| format!("creating a temporary file beside {}", path.display()))?;

    {
        let mut file = temporary.as_file();
        file.write_all(contents.as_bytes())
            .with_context(|| format!("writing {}", temporary.path().display()))?;
        file.sync_all()
            .with_context(|| format!("flushing {}", temporary.path().display()))?;
    }

    // A fresh temporary file is private to its creator; the file it replaces keeps the mode it had.
    #[cfg(unix)]
    {
        let permissions = fs::metadata(path)
            .with_context(|| format!("reading {}", path.display()))?
            .permissions();
        fs::set_permissions(temporary.path(), permissions)
            .with_context(|| format!("chmod {}", temporary.path().display()))?;
    }

    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("replacing {}", path.display()))?;

    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    let mut permissions = metadata.permissions();
    let mode = permissions.mode();

    // Only where reading is already allowed: a file readable by its owner alone stays that way.
    permissions.set_mode(mode | ((mode & 0o444) >> 2));

    fs::set_permissions(path, permissions).with_context(|| format!("chmod {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    // Windows has no bit to set; the rule that produces this fix never fires there.
    Ok(())
}

/// Whether anything is left to do, for the loop that re-lints after fixing.
pub fn has_fixes(report: &Report) -> bool {
    report.fixable() > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Location, Message, Reference, Severity, SkillReport, Source};

    fn fix(start: usize, end: usize, replacement: &str) -> Fix {
        Fix {
            start,
            end,
            replacement: replacement.into(),
            description: "a fix".into(),
        }
    }

    fn message(file: &str, fix: Option<Fix>) -> Message {
        Message {
            rule: "a/rule".into(),
            severity: Severity::Warning,
            message: "something".into(),
            advice: "do something".into(),
            location: Location::at(1, 1),
            source: Source::Static,
            file: file.into(),
            fix,
            reference: Reference {
                title: "t".into(),
                url: "https://example.com".into(),
            },
            confidence: 1.0,
        }
    }

    #[test]
    fn a_single_fix_is_spliced_in() {
        let fixes = [fix(6, 11, "there")];
        let borrowed: Vec<&Fix> = fixes.iter().collect();
        let refs: Vec<&&Fix> = borrowed.iter().collect();

        let (patched, applied, deferred) = patch("hello world", &refs);

        assert_eq!(patched, "hello there");
        assert_eq!((applied, deferred), (1, 0));
    }

    #[test]
    fn several_fixes_are_applied_last_first_so_offsets_stay_valid() {
        let fixes = [fix(0, 5, "goodbye"), fix(6, 11, "everyone")];
        let borrowed: Vec<&Fix> = fixes.iter().collect();
        let refs: Vec<&&Fix> = borrowed.iter().collect();

        let (patched, applied, _) = patch("hello world", &refs);

        assert_eq!(patched, "goodbye everyone");
        assert_eq!(applied, 2);
    }

    #[test]
    fn overlapping_fixes_leave_the_earlier_one_for_the_next_pass() {
        let fixes = [fix(0, 11, "entirely new text"), fix(6, 11, "everyone")];
        let borrowed: Vec<&Fix> = fixes.iter().collect();
        let refs: Vec<&&Fix> = borrowed.iter().collect();

        let (patched, applied, deferred) = patch("hello world", &refs);

        // The one furthest into the file goes in first, so offsets before it stay valid. The one
        // that overlaps it was computed against text that has now moved, so it waits.
        assert_eq!(patched, "hello everyone");
        assert_eq!((applied, deferred), (1, 1));
    }

    #[test]
    fn a_fix_pointing_outside_the_text_is_refused_rather_than_panicking() {
        let fixes = [fix(0, 900, "nonsense")];
        let borrowed: Vec<&Fix> = fixes.iter().collect();
        let refs: Vec<&&Fix> = borrowed.iter().collect();

        let (patched, applied, deferred) = patch("short", &refs);

        assert_eq!(patched, "short");
        assert_eq!((applied, deferred), (0, 1));
    }

    #[test]
    fn a_fix_landing_mid_character_is_refused_rather_than_corrupting_the_file() {
        // "é" is two bytes, so an offset of 1 is inside it.
        let fixes = [fix(1, 2, "x")];
        let borrowed: Vec<&Fix> = fixes.iter().collect();
        let refs: Vec<&&Fix> = borrowed.iter().collect();

        let (patched, applied, deferred) = patch("é", &refs);

        assert_eq!(patched, "é");
        assert_eq!((applied, deferred), (0, 1));
    }

    #[test]
    fn applying_writes_the_file_and_counts_what_it_did() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("SKILL.md");
        fs::write(&path, "Read scripts\\notes.md.\n").unwrap();

        let report = Report {
            skills: vec![SkillReport {
                path: temporary.path().display().to_string(),
                name: "a".into(),
                messages: vec![message(
                    path.to_str().unwrap(),
                    Some(fix(0, 22, "Read scripts/notes.md.")),
                )],
                notes: vec![],
            }],
            fixed: 0,
            notes: Vec::new(),
        };

        let applied = apply(&report);

        assert_eq!(
            applied,
            Applied {
                files: 1,
                fixes: 1,
                deferred: 0,
                failed: Vec::new(),
            }
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "Read scripts/notes.md.\n"
        );
    }

    /// A report whose one skill carries one fix for the given path.
    fn report_fixing(path: &Path, fix: Fix) -> Report {
        Report {
            skills: vec![SkillReport {
                path: path.parent().unwrap().display().to_string(),
                name: "a".into(),
                messages: vec![message(path.to_str().unwrap(), Some(fix))],
                notes: vec![],
            }],
            fixed: 0,
            notes: Vec::new(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_written_is_reported_and_the_others_still_get_fixed() {
        // Reproduces https://github.com/MaximeGaudin/slint/issues/227: one write failure used to
        // abort the whole run, leaving the files fixed before it with nothing in the report.
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let open = temporary.path().join("open.md");
        fs::write(&open, "Read scripts\\notes.md.\n").unwrap();

        let locked_directory = temporary.path().join("locked");
        fs::create_dir(&locked_directory).unwrap();
        let locked = locked_directory.join("locked.md");
        fs::write(&locked, "Read scripts\\notes.md.\n").unwrap();
        // Replacing a file needs a new file beside it, so a directory that refuses one is what
        // makes the write fail; the file's own mode would not.
        fs::set_permissions(&locked_directory, fs::Permissions::from_mode(0o555)).unwrap();

        let report = Report {
            skills: vec![
                SkillReport {
                    path: temporary.path().display().to_string(),
                    name: "locked".into(),
                    messages: vec![message(
                        locked.to_str().unwrap(),
                        Some(fix(0, 22, "Read scripts/notes.md.")),
                    )],
                    notes: vec![],
                },
                SkillReport {
                    path: temporary.path().display().to_string(),
                    name: "open".into(),
                    messages: vec![message(
                        open.to_str().unwrap(),
                        Some(fix(0, 22, "Read scripts/notes.md.")),
                    )],
                    notes: vec![],
                },
            ],
            fixed: 0,
            notes: Vec::new(),
        };

        let applied = apply(&report);
        fs::set_permissions(&locked_directory, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(applied.fixes, 1, "the writable file was fixed anyway");
        assert_eq!(
            fs::read_to_string(&open).unwrap(),
            "Read scripts/notes.md.\n"
        );
        assert_eq!(applied.failed.len(), 1, "{:?}", applied.failed);
        assert!(applied.failed[0].file.ends_with("locked.md"));
        assert!(
            applied.failed[0].reason.contains("Permission denied"),
            "{:?}",
            applied.failed[0].reason
        );
        assert!(
            fs::read_to_string(&locked).unwrap().contains('\\'),
            "the file that could not be written is exactly as it was"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_fix_replaces_the_file_rather_than_rewriting_it_in_place() {
        // The observable signature of temp-file + rename: what is on disk afterwards is a new
        // file. A truncate-and-write keeps the old one, so a crash mid-write leaves it half
        // written with the original gone.
        use std::os::unix::fs::MetadataExt;

        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("SKILL.md");
        fs::write(&path, "Read scripts\\notes.md.\n").unwrap();
        let before = fs::metadata(&path).unwrap().ino();

        apply(&report_fixing(&path, fix(0, 22, "Read scripts/notes.md.")));

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "Read scripts/notes.md.\n"
        );
        assert_ne!(
            before,
            fs::metadata(&path).unwrap().ino(),
            "the file was replaced, not truncated"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_file_keeps_the_permissions_it_had() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("SKILL.md");
        fs::write(&path, "Read scripts\\notes.md.\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        apply(&report_fixing(&path, fix(0, 22, "Read scripts/notes.md.")));

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644, "the replacement keeps the file's mode");
    }

    #[test]
    fn applying_a_fix_leaves_no_temporary_files_behind() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("SKILL.md");
        fs::write(&path, "Read scripts\\notes.md.\n").unwrap();

        apply(&report_fixing(&path, fix(0, 22, "Read scripts/notes.md.")));

        let entries = fs::read_dir(temporary.path()).unwrap().count();
        assert_eq!(entries, 1, "only the skill file remains");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "Read scripts/notes.md.\n"
        );
    }

    #[test]
    fn a_report_with_nothing_fixable_writes_nothing() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("SKILL.md");
        fs::write(&path, "unchanged\n").unwrap();

        let report = Report {
            skills: vec![SkillReport {
                path: temporary.path().display().to_string(),
                name: "a".into(),
                messages: vec![message(path.to_str().unwrap(), None)],
                notes: vec![],
            }],
            fixed: 0,
            notes: Vec::new(),
        };

        assert_eq!(apply(&report), Applied::default());
        assert!(!has_fixes(&report));
        assert_eq!(fs::read_to_string(&path).unwrap(), "unchanged\n");
    }

    #[cfg(unix)]
    #[test]
    fn an_empty_fix_over_an_empty_range_sets_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("cull.py");
        fs::write(&path, "#!/usr/bin/env python3\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let report = Report {
            skills: vec![SkillReport {
                path: temporary.path().display().to_string(),
                name: "a".into(),
                messages: vec![message(path.to_str().unwrap(), Some(fix(0, 0, "")))],
                notes: vec![],
            }],
            fixed: 0,
            notes: Vec::new(),
        };

        apply(&report);

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "readable by all, now runnable by all");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "#!/usr/bin/env python3\n",
            "the bytes are untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_private_script_stays_private_when_it_is_made_runnable() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("cull.py");
        fs::write(&path, "#!/usr/bin/env python3\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        make_executable(&path).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "only the owner gains anything");
    }
}
