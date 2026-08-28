//! Finding skills on disk, and reading one.
//!
//! A skill is a directory holding `SKILL.md` and whatever that file names. The parse is deliberately
//! forgiving — a linter that refuses to read a malformed skill cannot tell anyone what is wrong with
//! it — so anything unparseable comes back as a note on the skill rather than an error from the
//! reader.

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

/// The document an agent is handed.
pub const SKILL_FILE: &str = "SKILL.md";

/// A file shipped beside the instructions.
#[derive(Debug, Clone, Serialize)]
pub struct BundledFile {
    /// Relative to the skill directory, with forward slashes on every platform.
    pub path: String,
    pub bytes: usize,
    pub executable: bool,
    /// `None` for anything that is not UTF-8 text: an image is a file to count, not to read.
    pub text: Option<String>,
}

/// One skill, read.
#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    /// The directory, as it was given on the command line.
    pub directory: PathBuf,
    /// `SKILL.md`, relative to the run, for reporting.
    pub document: String,
    /// The whole file, frontmatter included. Fixes are byte offsets into this.
    pub source: String,
    /// Everything before the body, so a line in the body can be turned into a line in the file.
    pub frontmatter_lines: usize,
    /// Whether a frontmatter block was found at all.
    pub has_frontmatter: bool,
    pub name: String,
    pub description: String,
    /// The instructions, without the frontmatter.
    pub body: String,
    /// Byte offset of the body inside `source`.
    pub body_offset: usize,
    /// Frontmatter keys other than name and description, in the order they were written.
    pub metadata: BTreeMap<String, String>,
    /// The YAML type each top-level frontmatter key parsed as, so a rule can tell a
    /// `description: true` was not written as a string.
    pub scalar_types: BTreeMap<String, String>,
    pub files: Vec<BundledFile>,
    /// What could not be read, said out loud rather than dropped.
    pub notes: Vec<String>,
}

impl Skill {
    /// The line in `SKILL.md` a body line falls on.
    pub fn document_line(&self, body_line: usize) -> usize {
        body_line + self.frontmatter_lines
    }

    /// Where a frontmatter key is written, for a finding about the description or the name.
    pub fn frontmatter_line(&self, key: &str) -> usize {
        for (index, line) in self.source.lines().enumerate() {
            if line.starts_with(&format!("{key}:")) {
                return index + 1;
            }
        }

        1
    }

    pub fn file(&self, path: &str) -> Option<&BundledFile> {
        self.files.iter().find(|one| one.path == path)
    }
}

/// Every skill under the given paths.
///
/// A path that is itself a skill directory is taken as one; anything else is walked. Directories
/// that are never a skill — `.git`, `node_modules`, `target` — are skipped without being configured,
/// because nobody has ever wanted them linted.
pub fn discover(paths: &[PathBuf], ignore: &GlobSet) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();

    for path in paths {
        if path.join(SKILL_FILE).is_file() {
            found.push(path.clone());
            continue;
        }

        if path.is_file() {
            // A path straight to a SKILL.md is the shape an editor integration sends.
            if path.file_name().and_then(|name| name.to_str()) == Some(SKILL_FILE)
                && let Some(parent) = path.parent()
            {
                found.push(parent.to_path_buf());
            }
            continue;
        }

        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !is_never_a_skill(entry.path()))
        {
            let entry = entry.with_context(|| format!("walking {}", path.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.file_name() != SKILL_FILE {
                continue;
            }

            if let Some(parent) = entry.path().parent() {
                found.push(parent.to_path_buf());
            }
        }
    }

    found.retain(|directory| !ignore.is_match(directory));
    found.sort();
    found.dedup();

    Ok(found)
}

fn is_never_a_skill(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "node_modules" | "target" | "dist" | ".venv" | "__pycache__")
    )
}

pub fn build_ignore(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();

    for pattern in patterns {
        builder.add(Glob::new(pattern).with_context(|| format!("bad ignore pattern: {pattern}"))?);
    }

    builder.build().context("building the ignore set")
}

/// Reads one skill directory.
pub fn read(directory: &Path) -> Result<Skill> {
    let document_path = directory.join(SKILL_FILE);
    let source = fs::read_to_string(&document_path)
        .with_context(|| format!("reading {}", document_path.display()))?;

    let mut skill = parse(&source);
    skill.directory = directory.to_path_buf();
    skill.document = format!("{}/{SKILL_FILE}", directory.display());

    if skill.name.is_empty() {
        // The directory name is what an agent would see if the frontmatter is silent, and it makes
        // every message about the skill say which skill.
        skill.name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
    }

    skill.files = read_bundle(directory)?;

    Ok(skill)
}

fn read_bundle(directory: &Path) -> Result<Vec<BundledFile>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !is_never_a_skill(entry.path()))
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.file_name() == SKILL_FILE && entry.path().parent() == Some(directory) {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(directory)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");

        let bytes = entry
            .metadata()
            .map(|meta| meta.len() as usize)
            .unwrap_or(0);
        let text = fs::read(entry.path())
            .ok()
            .and_then(|raw| String::from_utf8(raw).ok());

        files.push(BundledFile {
            path: relative,
            bytes,
            executable: is_executable(entry.path()),
            text,
        });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    // Windows has no bit to read. Reporting every script as runnable is the kinder mistake: the
    // alternative is a warning on every file on a platform that cannot act on it.
    true
}

/// Splits a `SKILL.md` into frontmatter and body.
///
/// The frontmatter is read with a real YAML parser (yaml-rust2), so block scalars, quoted
/// values, lists, and nested maps behave the way a conforming Agent Skills loader reads them.
/// The parse stays forgiving — anything unparseable becomes a note on the skill rather than an
/// error — and no value is silently dropped: non-scalar values are kept as YAML text under
/// their key.
pub fn parse(source: &str) -> Skill {
    let mut skill = Skill {
        directory: PathBuf::new(),
        document: SKILL_FILE.to_string(),
        source: source.to_string(),
        frontmatter_lines: 0,
        has_frontmatter: false,
        name: String::new(),
        description: String::new(),
        body: source.to_string(),
        body_offset: 0,
        metadata: BTreeMap::new(),
        scalar_types: BTreeMap::new(),
        files: Vec::new(),
        notes: Vec::new(),
    };

    let normalised = source.strip_prefix('\u{feff}').unwrap_or(source);
    if !normalised.starts_with("---") {
        skill.notes.push(
            "No frontmatter: the name and description an agent selects on are not declared.".into(),
        );
        return skill;
    }

    let mut lines = normalised.lines();
    lines.next();

    let mut consumed = 1;
    let mut closed = false;
    let mut frontmatter = String::new();

    for line in lines {
        consumed += 1;

        if line.trim_end() == "---" {
            closed = true;
            break;
        }

        frontmatter.push_str(line);
        frontmatter.push('\n');
    }

    if !closed {
        skill.notes.push(
            "The frontmatter block is never closed, so the whole file reads as metadata.".into(),
        );
        return skill;
    }

    skill.has_frontmatter = true;
    skill.frontmatter_lines = consumed;

    match YamlLoader::load_from_str(&frontmatter) {
        Ok(documents) => read_frontmatter(&mut skill, documents.first()),
        Err(error) => skill
            .notes
            .push(format!("The frontmatter is not valid YAML: {error}")),
    }

    let offset = byte_offset_of_line(normalised, consumed);
    skill.body_offset = offset;
    skill.body = normalised[offset..].to_string();

    skill
}

/// Reads one parsed YAML document into the skill's fields.
fn read_frontmatter(skill: &mut Skill, document: Option<&Yaml>) {
    let Some(Yaml::Hash(mapping)) = document else {
        if document.is_some() {
            skill
                .notes
                .push("The frontmatter is not a YAML mapping of keys to values.".into());
        }
        return;
    };

    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            skill
                .notes
                .push("Frontmatter keys must be strings; a non-string key was skipped.".into());
            continue;
        };

        skill
            .scalar_types
            .insert(key.to_string(), yaml_type(value).to_string());

        match key {
            "name" => {
                if let Some(text) = scalar_string(value) {
                    skill.name = text;
                }
            }
            "description" => {
                if let Some(text) = scalar_string(value) {
                    skill.description = text;
                }
            }
            "metadata" => flatten_map("metadata", value, &mut skill.metadata),
            other => {
                let rendered = scalar_string(value)
                    .or_else(|| tool_list(value))
                    .unwrap_or_else(|| rendered_yaml(value));
                skill.metadata.insert(other.to_string(), rendered);
            }
        }
    }
}

/// The text a conforming loader hands out for a scalar, or `None` when the value is not one.
/// A null reads as empty, and a number or boolean keeps the text an author wrote.
fn scalar_string(value: &Yaml) -> Option<String> {
    match value {
        Yaml::String(text) => Some(text.clone()),
        Yaml::Integer(number) => Some(number.to_string()),
        Yaml::Real(number) => Some(number.clone()),
        Yaml::Boolean(flag) => Some(flag.to_string()),
        Yaml::Null => Some(String::new()),
        _ => None,
    }
}

/// The YAML type a value parsed as, in the words a rule reports to an author.
fn yaml_type(value: &Yaml) -> &'static str {
    match value {
        Yaml::String(_) => "string",
        Yaml::Integer(_) | Yaml::Real(_) => "number",
        Yaml::Boolean(_) => "boolean",
        Yaml::Null => "null",
        Yaml::Array(_) => "sequence",
        Yaml::Hash(_) => "mapping",
        _ => "invalid",
    }
}

/// Flattens a nested YAML mapping into dotted keys (`metadata.author`), so every pair stays
/// visible to the rules that read frontmatter keys.
fn flatten_map(prefix: &str, value: &Yaml, into: &mut BTreeMap<String, String>) {
    match value {
        Yaml::Hash(mapping) => {
            for (key, nested) in mapping {
                if let Some(key) = key.as_str() {
                    flatten_map(&format!("{prefix}.{key}"), nested, into);
                }
            }
        }
        scalar => {
            let rendered = scalar_string(scalar).unwrap_or_else(|| rendered_yaml(scalar));
            into.insert(prefix.to_string(), rendered);
        }
    }
}

/// Tool lists are sequences of tool names; whether an author writes a flow sequence or a block
/// list, the rules check membership in a space-separated list.
fn tool_list(value: &Yaml) -> Option<String> {
    let items = match value {
        Yaml::Array(items) => items
            .iter()
            .map(scalar_string)
            .collect::<Option<Vec<_>>>()?,
        Yaml::String(text) => vec![text.clone()],
        _ => return None,
    };

    Some(
        items
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Values that are not scalars stay visible as YAML text rather than being dropped.
fn rendered_yaml(value: &Yaml) -> String {
    let mut text = String::new();

    if YamlEmitter::new(&mut text).dump(value).is_err() {
        return String::new();
    }

    text
}

/// The byte offset where the given 0-based line index starts.
fn byte_offset_of_line(text: &str, line_index: usize) -> usize {
    let mut seen = 0;

    for (offset, character) in text.char_indices() {
        if seen == line_index {
            return offset;
        }
        if character == '\n' {
            seen += 1;
        }
    }

    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "---\nname: photo-culling\ndescription: Culls a shoot. Use when triaging RAW files.\nlicense: MIT\n---\n\n## Culling\n\n1. Import.\n";

    #[test]
    fn reads_the_frontmatter_an_agent_selects_on() {
        let skill = parse(DOCUMENT);

        assert!(skill.has_frontmatter);
        assert_eq!(skill.name, "photo-culling");
        assert_eq!(
            skill.description,
            "Culls a shoot. Use when triaging RAW files."
        );
        assert_eq!(skill.metadata.get("license"), Some(&"MIT".to_string()));
    }

    #[test]
    fn the_body_starts_after_the_frontmatter() {
        let skill = parse(DOCUMENT);

        assert!(skill.body.starts_with("\n## Culling"));
        assert_eq!(&skill.source[skill.body_offset..], skill.body);
    }

    #[test]
    fn a_body_line_maps_back_to_the_line_in_the_file() {
        let skill = parse(DOCUMENT);

        // "## Culling" is line 2 of the body and line 7 of the document.
        assert_eq!(skill.document_line(2), 7);
    }

    #[test]
    fn finds_the_line_a_frontmatter_key_is_written_on() {
        let skill = parse(DOCUMENT);

        assert_eq!(skill.frontmatter_line("name"), 2);
        assert_eq!(skill.frontmatter_line("description"), 3);
    }

    #[test]
    fn a_folded_description_is_folded_like_a_real_yaml_parser() {
        let skill = parse(
            "---\nname: a\ndescription: >-\n  First half of the sentence,\n  and the rest of it.\n---\n\nBody.\n",
        );

        assert_eq!(
            skill.description,
            "First half of the sentence, and the rest of it."
        );
    }

    #[test]
    fn a_folded_block_scalar_indicator_does_not_leak_into_the_description() {
        // Regression for #64: `>` and `|` are YAML block-scalar indicators, not text. A real YAML
        // parser folds the lines that follow and never keeps the indicator character.
        let skill = parse(
            "---\nname: demo-skill\ndescription: >\n  Analyzes call transcripts and flags compliance issues.\n  Use when reviewing support calls for compliance keywords.\n---\n\nBody.\n",
        );

        assert_eq!(
            skill.description,
            "Analyzes call transcripts and flags compliance issues. Use when reviewing support calls for compliance keywords.\n"
        );
    }

    #[test]
    fn a_literal_block_scalar_keeps_its_line_breaks() {
        let skill = parse("---\nname: a\ndescription: |\n  Line one.\n  Line two.\n---\n\nBody.\n");

        assert_eq!(skill.description, "Line one.\nLine two.\n");
    }

    #[test]
    fn a_nested_metadata_map_is_parsed_into_pairs() {
        // Regression for #123: `metadata` is a YAML mapping of key-value pairs, not a text blob.
        let skill = parse(
            "---\nname: a\ndescription: b\nmetadata:\n  author: X\n  version: 1.0\n---\n\nBody.\n",
        );

        assert_eq!(
            skill.metadata.get("metadata.author"),
            Some(&"X".to_string())
        );
        assert_eq!(
            skill.metadata.get("metadata.version"),
            Some(&"1.0".to_string())
        );
    }

    #[test]
    fn a_boolean_description_is_recorded_with_its_type() {
        // Regression for #94: the parser must not quietly stringify a non-string scalar.
        let skill = parse("---\nname: a\ndescription: true\n---\n\nBody.\n");

        assert_eq!(skill.description, "true");
        assert_eq!(
            skill.scalar_types.get("description"),
            Some(&"boolean".to_string())
        );
    }

    #[test]
    fn a_number_name_is_recorded_with_its_type() {
        let skill = parse("---\nname: 12345\ndescription: b\n---\n\nBody.\n");

        assert_eq!(skill.name, "12345");
        assert_eq!(skill.scalar_types.get("name"), Some(&"number".to_string()));
    }

    #[test]
    fn a_sequence_description_is_recorded_as_a_sequence() {
        let skill = parse("---\nname: a\ndescription:\n  - first\n  - second\n---\n\nBody.\n");

        assert_eq!(skill.description, "");
        assert_eq!(
            skill.scalar_types.get("description"),
            Some(&"sequence".to_string())
        );
    }

    #[test]
    fn string_values_record_their_type_as_string() {
        let skill = parse(DOCUMENT);

        assert_eq!(skill.scalar_types.get("name"), Some(&"string".to_string()));
        assert_eq!(
            skill.scalar_types.get("description"),
            Some(&"string".to_string())
        );
    }

    #[test]
    fn a_document_with_no_frontmatter_says_so_instead_of_failing() {
        let skill = parse("# Just a heading\n\nSome instructions.\n");

        assert!(!skill.has_frontmatter);
        assert!(
            skill
                .notes
                .iter()
                .any(|note| note.contains("No frontmatter"))
        );
        assert_eq!(skill.body, "# Just a heading\n\nSome instructions.\n");
    }

    #[test]
    fn an_unclosed_frontmatter_block_is_reported_rather_than_guessed_at() {
        let skill = parse("---\nname: a\ndescription: b\n\n## Body\n");

        assert!(!skill.has_frontmatter);
        assert!(skill.notes.iter().any(|note| note.contains("never closed")));
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_frontmatter() {
        let skill = parse("\u{feff}---\nname: a\ndescription: b\n---\n\nBody.\n");

        assert!(skill.has_frontmatter);
        assert_eq!(skill.name, "a");
    }

    #[test]
    fn quotes_around_a_value_are_not_part_of_it() {
        let skill = parse("---\nname: \"quoted\"\ndescription: 'also quoted'\n---\n\nBody.\n");

        assert_eq!(skill.name, "quoted");
        assert_eq!(skill.description, "also quoted");
    }

    #[test]
    fn discovery_finds_a_directory_that_is_itself_a_skill() {
        let temporary = tempfile::tempdir().unwrap();
        let skill = temporary.path().join("photo-culling");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join(SKILL_FILE), DOCUMENT).unwrap();

        let found = discover(std::slice::from_ref(&skill), &GlobSet::empty()).unwrap();
        assert_eq!(found, vec![skill]);
    }

    #[test]
    fn discovery_walks_a_tree_and_skips_what_is_never_a_skill() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();

        for name in ["one", "two"] {
            let directory = root.join("skills").join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(SKILL_FILE), DOCUMENT).unwrap();
        }

        let hidden = root.join("node_modules").join("three");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join(SKILL_FILE), DOCUMENT).unwrap();

        let found = discover(&[root.to_path_buf()], &GlobSet::empty()).unwrap();

        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .all(|path| !path.to_string_lossy().contains("node_modules"))
        );
    }

    #[test]
    fn discovery_honours_an_ignore_pattern() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();

        for name in ["kept", "fixtures"] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(SKILL_FILE), DOCUMENT).unwrap();
        }

        let ignore = build_ignore(&["**/fixtures".to_string()]).unwrap();
        let found = discover(&[root.to_path_buf()], &ignore).unwrap();

        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("kept"));
    }

    #[test]
    fn a_malformed_ignore_glob_names_the_pattern_and_fails() {
        let failure = match build_ignore(&["[invalid-glob".to_string()]) {
            Err(failure) => failure.to_string(),
            Ok(_) => panic!("a malformed glob has to be an error"),
        };

        assert!(failure.contains("bad ignore pattern"));
        assert!(failure.contains("[invalid-glob"));
    }

    #[test]
    fn a_path_to_the_document_itself_resolves_to_its_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("photo-culling");
        fs::create_dir_all(&directory).unwrap();
        let document = directory.join(SKILL_FILE);
        fs::write(&document, DOCUMENT).unwrap();

        let found = discover(&[document], &GlobSet::empty()).unwrap();
        assert_eq!(found, vec![directory]);
    }

    #[test]
    fn reading_a_skill_picks_up_its_bundle() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("photo-culling");
        fs::create_dir_all(directory.join("scripts")).unwrap();
        fs::write(directory.join(SKILL_FILE), DOCUMENT).unwrap();
        fs::write(
            directory.join("scripts/cull.py"),
            "#!/usr/bin/env python3\n",
        )
        .unwrap();

        let skill = read(&directory).unwrap();

        assert_eq!(skill.files.len(), 1);
        assert_eq!(skill.files[0].path, "scripts/cull.py");
        assert!(skill.files[0].text.as_deref().unwrap().starts_with("#!"));
    }

    #[test]
    fn a_skill_with_no_name_in_its_frontmatter_is_named_after_its_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("named-by-folder");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(SKILL_FILE),
            "---\ndescription: A thing.\n---\n\nBody.\n",
        )
        .unwrap();

        let skill = read(&directory).unwrap();
        assert_eq!(skill.name, "named-by-folder");
    }
}
