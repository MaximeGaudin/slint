//! Rules about the files a skill ships.
//!
//! These are the ones a diff cannot produce: whether a path in the instructions is a path that
//! exists, and whether a file that exists is one anything reads.

use regex::Regex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

use crate::diagnostics::{Fix, Location, Reference, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, sources};

/// A relative path that looks like it means a bundled file.
static BUNDLED_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[\s`(\x22'\[])((?:scripts|references|reference|assets|templates|data)/[\w./-]+)",
    )
    .expect("the bundled reference pattern compiles")
});

static PYTHON_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:import|from)\s+([A-Za-z_][A-Za-z0-9_]*)")
        .expect("the import pattern compiles")
});

/// Every top-level module of the CPython standard library, from 3.8 through 3.14 (the names from
/// `sys.stdlib_module_names`, plus the ones 3.10 and 3.12 dropped).
const PYTHON_STDLIB: [&str; 320] = [
    "__future__",
    "_abc",
    "_aix_support",
    "_android_support",
    "_apple_support",
    "_ast",
    "_ast_unparse",
    "_asyncio",
    "_bisect",
    "_blake2",
    "_bz2",
    "_codecs",
    "_codecs_cn",
    "_codecs_hk",
    "_codecs_iso2022",
    "_codecs_jp",
    "_codecs_kr",
    "_codecs_tw",
    "_collections",
    "_collections_abc",
    "_colorize",
    "_compat_pickle",
    "_contextvars",
    "_csv",
    "_ctypes",
    "_curses",
    "_curses_panel",
    "_datetime",
    "_dbm",
    "_decimal",
    "_elementtree",
    "_frozen_importlib",
    "_frozen_importlib_external",
    "_functools",
    "_gdbm",
    "_hashlib",
    "_heapq",
    "_hmac",
    "_imp",
    "_interpchannels",
    "_interpqueues",
    "_interpreters",
    "_io",
    "_ios_support",
    "_json",
    "_locale",
    "_lsprof",
    "_lzma",
    "_markupbase",
    "_md5",
    "_multibytecodec",
    "_multiprocessing",
    "_opcode",
    "_opcode_metadata",
    "_operator",
    "_osx_support",
    "_overlapped",
    "_pickle",
    "_posixshmem",
    "_posixsubprocess",
    "_py_abc",
    "_py_warnings",
    "_pydatetime",
    "_pydecimal",
    "_pyio",
    "_pylong",
    "_pyrepl",
    "_queue",
    "_random",
    "_remote_debugging",
    "_scproxy",
    "_sha1",
    "_sha2",
    "_sha3",
    "_signal",
    "_sitebuiltins",
    "_socket",
    "_sqlite3",
    "_sre",
    "_ssl",
    "_stat",
    "_statistics",
    "_string",
    "_strptime",
    "_struct",
    "_suggestions",
    "_symtable",
    "_sysconfig",
    "_thread",
    "_threading_local",
    "_tkinter",
    "_tokenize",
    "_tracemalloc",
    "_types",
    "_typing",
    "_uuid",
    "_warnings",
    "_weakref",
    "_weakrefset",
    "_winapi",
    "_wmi",
    "_zoneinfo",
    "_zstd",
    "abc",
    "aifc",
    "annotationlib",
    "antigravity",
    "argparse",
    "array",
    "ast",
    "asynchat",
    "asyncio",
    "asyncore",
    "atexit",
    "audioop",
    "base64",
    "bdb",
    "binascii",
    "bisect",
    "builtins",
    "bz2",
    "cProfile",
    "calendar",
    "cgi",
    "cgitb",
    "chunk",
    "cmath",
    "cmd",
    "code",
    "codecs",
    "codeop",
    "collections",
    "colorsys",
    "compileall",
    "compression",
    "concurrent",
    "configparser",
    "contextlib",
    "contextvars",
    "copy",
    "copyreg",
    "csv",
    "ctypes",
    "curses",
    "dataclasses",
    "datetime",
    "dbm",
    "decimal",
    "difflib",
    "dis",
    "distutils",
    "doctest",
    "email",
    "encodings",
    "ensurepip",
    "enum",
    "errno",
    "faulthandler",
    "fcntl",
    "filecmp",
    "fileinput",
    "fnmatch",
    "fractions",
    "ftplib",
    "functools",
    "gc",
    "genericpath",
    "getopt",
    "getpass",
    "gettext",
    "glob",
    "graphlib",
    "grp",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "idlelib",
    "imaplib",
    "imp",
    "importlib",
    "inspect",
    "io",
    "ipaddress",
    "itertools",
    "json",
    "keyword",
    "lib2to3",
    "linecache",
    "locale",
    "logging",
    "lzma",
    "mailbox",
    "mailcap",
    "marshal",
    "math",
    "mimetypes",
    "mmap",
    "modulefinder",
    "msilib",
    "msvcrt",
    "multiprocessing",
    "netrc",
    "nis",
    "nntplib",
    "nt",
    "ntpath",
    "nturl2path",
    "numbers",
    "opcode",
    "operator",
    "optparse",
    "os",
    "ossaudiodev",
    "pathlib",
    "pdb",
    "pickle",
    "pickletools",
    "pipes",
    "pkgutil",
    "platform",
    "plistlib",
    "poplib",
    "posix",
    "posixpath",
    "pprint",
    "profile",
    "pstats",
    "pty",
    "pwd",
    "py_compile",
    "pyclbr",
    "pydoc",
    "pydoc_data",
    "pyexpat",
    "queue",
    "quopri",
    "random",
    "re",
    "readline",
    "reprlib",
    "resource",
    "rlcompleter",
    "runpy",
    "sched",
    "secrets",
    "select",
    "selectors",
    "shelve",
    "shlex",
    "shutil",
    "signal",
    "site",
    "smtpd",
    "smtplib",
    "sndhdr",
    "socket",
    "socketserver",
    "spwd",
    "sqlite3",
    "sre_compile",
    "sre_constants",
    "sre_parse",
    "ssl",
    "stat",
    "statistics",
    "string",
    "stringprep",
    "struct",
    "subprocess",
    "sunau",
    "symtable",
    "sys",
    "sysconfig",
    "syslog",
    "tabnanny",
    "tarfile",
    "telnetlib",
    "tempfile",
    "termios",
    "textwrap",
    "this",
    "threading",
    "time",
    "timeit",
    "tkinter",
    "token",
    "tokenize",
    "tomllib",
    "trace",
    "traceback",
    "tracemalloc",
    "tty",
    "turtle",
    "turtledemo",
    "types",
    "typing",
    "unicodedata",
    "unittest",
    "urllib",
    "uu",
    "uuid",
    "venv",
    "warnings",
    "wave",
    "weakref",
    "webbrowser",
    "winreg",
    "winsound",
    "wsgiref",
    "xdrlib",
    "xml",
    "xmlrpc",
    "zipapp",
    "zipfile",
    "zipimport",
    "zlib",
    "zoneinfo",
];

static NO_DANGLING: RuleMeta = RuleMeta {
    name: "bundle/no-dangling-path",
    summary: "Every path mentioned in the instructions must exist in the skill folder.",
    rationale: "The agent will try to open that path after unpacking the skill. If the file is missing, the step fails and the agent has to guess.",
    advice: "Either add the missing file under the skill directory, or remove that path from the instructions.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static UNUSED_FILE: RuleMeta = RuleMeta {
    name: "bundle/unused-file",
    summary: "Every file in the skill folder should be referenced from the instructions (or another linked file).",
    rationale: "Unreferenced files are still downloaded with the skill but never read — wasted size and a sign the docs are out of date.",
    advice: "Mention the file from SKILL.md (or remove the file if it is leftover).",
    default_severity: Severity::Warning,
    // Deliberately not fixable: deleting a file is not something to do in a batch, and a path
    // normalised in the same run can make this very file referenced.
    fixable: false,
    needs_model: false,
    reference_title: sources::PAPER.0,
    reference_url: sources::PAPER.1,
};

static EXECUTABLE: RuleMeta = RuleMeta {
    name: "bundle/executable-script",
    summary: "Scripts that start with a shebang (#!) must be marked executable.",
    rationale: "A shebang means the file is meant to be run as a program. Without the executable bit, the first run fails with a permission error.",
    advice: "Make the script executable (for example: chmod +x path/to/script).",
    default_severity: Severity::Warning,
    fixable: true,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static FLAT_REFERENCES: RuleMeta = RuleMeta {
    name: "bundle/flat-references",
    summary: "Link bundled files from SKILL.md, not only from other bundled files.",
    rationale: "Agents usually read SKILL.md first and may only partially read secondary files. A file reachable only through another file is often skipped.",
    advice: "Add a direct reference to that file from SKILL.md (path or link in the steps).",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static CONTENTS_LIST: RuleMeta = RuleMeta {
    name: "bundle/contents-list",
    summary: "Long reference files should start with a table of contents.",
    rationale: "Agents often preview only the top of a long file. A contents list at the top still shows what else is inside after a partial read.",
    advice: "Add a short contents list at the top of the file, using the headings you already have.",
    default_severity: Severity::Info,
    fixable: true,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static DECLARED_DEPENDENCIES: RuleMeta = RuleMeta {
    name: "bundle/declared-dependencies",
    summary: "Third-party packages a script imports must be named in the install instructions.",
    rationale: "If the agent runs the script without installing those packages, the import fails and the skill stops mid-task.",
    advice: "In SKILL.md, at the step that runs the script, name each third-party package and the install command (for example: pip install requests).",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static SCRIPT_PREREQUISITES: RuleMeta = RuleMeta {
    name: "bundle/script-prerequisites",
    summary: "Skills that ship scripts must document runtime needs in a Prerequisites section.",
    rationale: "Scripts often need host tools, interpreters, or network access that only show up when reading the script. Agents following SKILL.md alone will miss those requirements and fail mid-run.",
    advice: "Add a Prerequisites (or Requirements / Compatibility) section that names the host tools, runtimes, and other non-obvious environment needs the scripts require.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

fn paths_in(text: &str) -> BTreeSet<String> {
    BUNDLED_REFERENCE
        .captures_iter(text)
        .filter_map(|captures| captures.get(1))
        .map(|found| {
            found
                .as_str()
                .trim_end_matches(|c: char| ".,;:)`'\"]".contains(c))
                .to_string()
        })
        .collect()
}

/// Prefixes accepted as bundled companion locations (Agent Skills optional dirs + slint extras).
const STANDARD_DIRECTORY_PREFIXES: &[&str] = &[
    "scripts/",
    "references/",
    "reference/",
    "assets/",
    "templates/",
    "data/",
];

fn is_in_standard_directory(path: &str) -> bool {
    STANDARD_DIRECTORY_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

const MISPLACED_ADVICE: &str = "Move it under one of: scripts/, references/, assets/ (slint also accepts: templates/, data/, reference/), then update the path in SKILL.md.";

struct NoDangling;
struct UnusedFile;
struct Executable;
struct FlatReferences;
struct ContentsList;
struct DeclaredDependencies;
struct ScriptPrerequisites;

impl Rule for NoDangling {
    fn meta(&self) -> &'static RuleMeta {
        &NO_DANGLING
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let bundled: BTreeSet<String> = context
            .skill
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        let body = context.skill.body.clone();

        for path in paths_in(&body) {
            if bundled.contains(&path) {
                continue;
            }

            let line = body
                .lines()
                .position(|line| line.contains(&path))
                .map(|index| context.skill.document_line(index + 1))
                .unwrap_or(1);

            context.report(
                format!("The instructions name {path}, which is not in the bundle"),
                Location::at(line, 1),
            );
        }
    }
}

impl Rule for UnusedFile {
    fn meta(&self) -> &'static RuleMeta {
        &UNUSED_FILE
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let mut reachable = paths_in(&context.skill.body);

        for file in &context.skill.files {
            if let Some(text) = &file.text {
                reachable.extend(paths_in(text));
            }
        }

        let paths: Vec<String> = context
            .skill
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();

        for path in paths {
            if !is_in_standard_directory(&path) {
                // Layout problem, not reachability — even when SKILL.md already links the file.
                context.report_in_file_with(
                    &path,
                    format!("{path} is outside the standard skill directories"),
                    Location::whole_file(),
                    MISPLACED_ADVICE,
                    Reference {
                        title: sources::OPTIONAL_DIRECTORIES.0.into(),
                        url: sources::OPTIONAL_DIRECTORIES.1.into(),
                    },
                );
                continue;
            }

            if reachable.contains(&path) {
                continue;
            }

            context.report_in_file(
                &path,
                format!("Nothing refers to {path}"),
                Location::whole_file(),
            );
        }
    }
}

impl Rule for Executable {
    fn meta(&self) -> &'static RuleMeta {
        &EXECUTABLE
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let candidates: Vec<String> = context
            .skill
            .files
            .iter()
            .filter(|file| {
                !file.executable
                    && file
                        .text
                        .as_deref()
                        .is_some_and(|text| text.starts_with("#!"))
            })
            .map(|file| file.path.clone())
            .collect();

        for path in candidates {
            context.report_fixable_in_file(
                &path,
                format!("{path} has a shebang and is not executable"),
                Location::at(1, 1),
                // A permission change rather than a text edit: the fixer recognises an empty
                // replacement over an empty range as "make this runnable".
                Fix {
                    start: 0,
                    end: 0,
                    replacement: String::new(),
                    description: "Sets the executable bit.".into(),
                },
            );
        }
    }
}

impl Rule for FlatReferences {
    fn meta(&self) -> &'static RuleMeta {
        &FLAT_REFERENCES
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let from_body = paths_in(&context.skill.body);
        let bundled: BTreeSet<String> = context
            .skill
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();

        let mut nested: Vec<(String, String)> = Vec::new();

        for file in &context.skill.files {
            let Some(text) = &file.text else { continue };

            for path in paths_in(text) {
                if path != file.path && bundled.contains(&path) && !from_body.contains(&path) {
                    nested.push((path, file.path.clone()));
                }
            }
        }

        for (path, from) in nested {
            context.report_in_file(
                &path,
                format!("{path} is only reachable through {from}"),
                Location::whole_file(),
            );
        }
    }
}

impl Rule for ContentsList {
    fn meta(&self) -> &'static RuleMeta {
        &CONTENTS_LIST
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let long_files: Vec<(String, String)> = context
            .skill
            .files
            .iter()
            .filter(|file| file.path.ends_with(".md"))
            .filter_map(|file| {
                file.text
                    .as_ref()
                    .map(|text| (file.path.clone(), text.clone()))
            })
            .filter(|(_, text)| text.lines().count() > 100 && !has_contents(text))
            .collect();

        for (path, text) in long_files {
            let lines = text.lines().count();

            match with_contents(&text) {
                Some(replacement) => context.report_fixable_in_file(
                    &path,
                    format!("{path} is {lines} lines with no contents list"),
                    Location::at(1, 1),
                    Fix {
                        start: 0,
                        end: text.len(),
                        replacement,
                        description: "Adds a contents list built from the headings already there."
                            .into(),
                    },
                ),
                None => context.report_in_file(
                    &path,
                    format!("{path} is {lines} lines with no contents list"),
                    Location::at(1, 1),
                ),
            }
        }
    }
}

impl Rule for DeclaredDependencies {
    fn meta(&self) -> &'static RuleMeta {
        &DECLARED_DEPENDENCIES
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let body = context.skill.body.to_ascii_lowercase();

        let missing: Vec<(String, String)> = context
            .skill
            .files
            .iter()
            .filter(|file| file.path.ends_with(".py"))
            .filter_map(|file| {
                file.text
                    .as_ref()
                    .map(|text| (file.path.clone(), text.clone()))
            })
            .flat_map(|(path, text)| {
                undeclared_imports(&text, &body)
                    .into_iter()
                    .map(move |module| (path.clone(), module))
            })
            .collect();

        for (path, module) in missing {
            context.report_in_file(
                &path,
                format!("{path} imports {module} and nothing says to install it"),
                Location::whole_file(),
            );
        }
    }
}

impl Rule for ScriptPrerequisites {
    fn meta(&self) -> &'static RuleMeta {
        &SCRIPT_PREREQUISITES
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let has_scripts = context
            .skill
            .files
            .iter()
            .any(|file| file.path.starts_with("scripts/"));

        if !has_scripts || has_prerequisites_section(&context.skill.body) {
            return;
        }

        let line = context.skill.frontmatter_lines.saturating_add(1).max(1);
        context.report(
            "This skill ships scripts but SKILL.md has no Prerequisites section",
            Location::at(line, 1),
        );
    }
}

fn has_prerequisites_section(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with('#') {
            return false;
        }

        let heading = trimmed.trim_start_matches('#').trim().to_ascii_lowercase();
        matches!(
            heading.as_str(),
            "prerequisites" | "requirements" | "compatibility"
        ) || heading.starts_with("prerequisites ")
            || heading.starts_with("requirements ")
            || heading.starts_with("compatibility ")
    })
}

fn undeclared_imports(script: &str, body: &str) -> Vec<String> {
    let mentioned = format!("{body}\n{}", script.to_ascii_lowercase());

    let mut modules: Vec<String> = PYTHON_IMPORT
        .captures_iter(script)
        .filter_map(|captures| captures.get(1))
        .map(|found| found.as_str().to_string())
        .filter(|module| !PYTHON_STDLIB.contains(&module.as_str()))
        .filter(|module| {
            let install = Regex::new(&format!(
                r"(?i)(pip install|uv (pip )?(install|add)|requirements)[^\n]*{}",
                regex::escape(module)
            ))
            .expect("the install pattern compiles");

            !install.is_match(&mentioned)
        })
        .collect();

    modules.sort();
    modules.dedup();
    modules
}

fn has_contents(text: &str) -> bool {
    text.lines().take(15).any(|line| {
        let lower = line.trim().to_ascii_lowercase();
        lower.starts_with("## contents")
            || lower.starts_with("# contents")
            || lower.starts_with("## table of contents")
            || lower.starts_with("## in this file")
    })
}

fn with_contents(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();

    let headings: Vec<String> = lines
        .iter()
        .filter(|line| line.starts_with("## "))
        .map(|line| format!("- {}", line.trim_start_matches("## ")))
        .collect();

    if headings.is_empty() {
        return None;
    }

    // After the title if there is one, which is where a reader looks for a contents list.
    let at = lines
        .iter()
        .position(|line| line.starts_with("# ") && !line.starts_with("## "))
        .map(|index| index + 1)
        .unwrap_or(0);

    let mut rebuilt: Vec<String> = lines[..at].iter().map(|line| line.to_string()).collect();
    rebuilt.push(String::new());
    rebuilt.push("## Contents".into());
    rebuilt.push(String::new());
    rebuilt.extend(headings);
    rebuilt.extend(lines[at..].iter().map(|line| line.to_string()));

    let mut joined = rebuilt.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }

    Some(joined)
}

static DANGLING_RULE: NoDangling = NoDangling;
static UNUSED_RULE: UnusedFile = UnusedFile;
static EXECUTABLE_RULE: Executable = Executable;
static FLAT_RULE: FlatReferences = FlatReferences;
static CONTENTS_RULE: ContentsList = ContentsList;
static DEPENDENCIES_RULE: DeclaredDependencies = DeclaredDependencies;
static PREREQUISITES_RULE: ScriptPrerequisites = ScriptPrerequisites;

pub fn rules() -> Vec<&'static dyn Rule> {
    vec![
        &DANGLING_RULE,
        &UNUSED_RULE,
        &EXECUTABLE_RULE,
        &FLAT_RULE,
        &CONTENTS_RULE,
        &DEPENDENCIES_RULE,
        &PREREQUISITES_RULE,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{check, good_skill, skill_with_body};
    use crate::skill::BundledFile;

    fn file(path: &str, text: &str, executable: bool) -> BundledFile {
        BundledFile {
            path: path.into(),
            bytes: text.len(),
            executable,
            text: Some(text.into()),
        }
    }

    #[test]
    fn a_skill_with_no_bundle_passes_every_rule_here() {
        let skill = good_skill();

        for rule in rules() {
            assert!(
                check(rule, &skill).is_empty(),
                "{} fired on a skill with no files",
                rule.meta().name
            );
        }
    }

    #[test]
    fn a_path_that_is_not_in_the_bundle_is_an_error() {
        let skill = skill_with_body("\n## Culling\n\nRun scripts/cull.py when you are done.\n");
        let messages = check(&DANGLING_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Error);
        assert!(messages[0].message.contains("scripts/cull.py"));
    }

    #[test]
    fn a_dangling_path_is_reported_on_the_line_that_names_it() {
        let skill = skill_with_body("\n## Culling\n\n1. Import.\n2. Run scripts/cull.py.\n");
        let messages = check(&DANGLING_RULE, &skill);

        assert_eq!(messages[0].location.line, skill.document_line(5));
    }

    #[test]
    fn a_referenced_file_that_exists_passes() {
        let mut skill = skill_with_body("\n## Culling\n\nRun scripts/cull.py when you are done.\n");
        skill
            .files
            .push(file("scripts/cull.py", "#!/usr/bin/env python3\n", true));

        assert!(check(&DANGLING_RULE, &skill).is_empty());
    }

    #[test]
    fn a_file_nothing_refers_to_is_reported() {
        let mut skill = good_skill();
        skill
            .files
            .push(file("references/formats.md", "# Formats\n", false));

        let messages = check(&UNUSED_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("references/formats.md"));
        assert!(messages[0].message.contains("Nothing refers to"));
        // Never fixable: deleting a file in a batch is how a fixed path loses the file it points at.
        assert!(messages[0].fix.is_none());
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/1 —
    /// a root companion already linked from SKILL.md must be diagnosed as
    /// layout (outside standard dirs), not as an unreferenced file.
    #[test]
    fn a_root_companion_linked_from_skill_md_is_reported_as_misplaced() {
        let mut skill = skill_with_body(
            "\n## Steps\n\nCopy the skeleton from [TEMPLATE.md](TEMPLATE.md) when writing a file.\n",
        );
        skill.files.push(file("TEMPLATE.md", "# Template\n", false));

        let messages = check(&UNUSED_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert!(
            messages[0]
                .message
                .contains("outside the standard skill directories"),
            "expected a layout diagnosis, got: {}",
            messages[0].message
        );
        assert!(
            !messages[0].message.contains("Nothing refers to"),
            "must not look like an unused-file diagnosis when the real issue is layout"
        );
        assert!(
            messages[0].advice.contains("scripts/")
                && messages[0].advice.contains("references/")
                && messages[0].advice.contains("assets/"),
            "advice should tell authors where to move the file: {}",
            messages[0].advice
        );
        assert!(
            messages[0].reference.url.contains("optional-directories"),
            "cite the Agent Skills optional directories: {}",
            messages[0].reference.url
        );
    }

    #[test]
    fn a_file_referenced_by_another_file_counts_as_used() {
        let mut skill = skill_with_body("\n## Culling\n\nRead references/formats.md first.\n");
        skill.files.push(file(
            "references/formats.md",
            "# Formats\n\nSee references/raw.md.\n",
            false,
        ));
        skill
            .files
            .push(file("references/raw.md", "# RAW\n", false));

        assert!(check(&UNUSED_RULE, &skill).is_empty());
    }

    #[test]
    fn a_file_reachable_only_through_another_file_is_reported() {
        let mut skill = skill_with_body("\n## Culling\n\nRead references/formats.md first.\n");
        skill.files.push(file(
            "references/formats.md",
            "# Formats\n\nSee references/raw.md.\n",
            false,
        ));
        skill
            .files
            .push(file("references/raw.md", "# RAW\n", false));

        let messages = check(&FLAT_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("references/raw.md"));
        assert!(messages[0].message.contains("references/formats.md"));
    }

    #[test]
    fn a_script_with_a_shebang_and_no_bit_is_reported_and_fixable() {
        let mut skill = skill_with_body("\n## Culling\n\nRun scripts/cull.py.\n");
        skill.files.push(file(
            "scripts/cull.py",
            "#!/usr/bin/env python3\nprint(1)\n",
            false,
        ));

        let messages = check(&EXECUTABLE_RULE, &skill);

        assert_eq!(messages.len(), 1);
        let fix = messages[0].fix.as_ref().unwrap();
        assert_eq!(
            (fix.start, fix.end),
            (0, 0),
            "a permission change, not an edit"
        );
    }

    #[test]
    fn a_script_already_marked_executable_passes() {
        let mut skill = skill_with_body("\n## Culling\n\nRun scripts/cull.py.\n");
        skill
            .files
            .push(file("scripts/cull.py", "#!/usr/bin/env python3\n", true));

        assert!(check(&EXECUTABLE_RULE, &skill).is_empty());
    }

    #[test]
    fn a_long_reference_file_gets_a_contents_list_written_from_its_headings() {
        let mut text = String::from("# Formats\n\n");
        for index in 0..40 {
            text.push_str(&format!("## Section {index}\n\nWords about it.\n\n"));
        }

        let mut skill = skill_with_body("\n## Culling\n\nRead references/formats.md.\n");
        skill
            .files
            .push(file("references/formats.md", &text, false));

        let messages = check(&CONTENTS_RULE, &skill);
        assert_eq!(messages.len(), 1);

        let fixed = &messages[0].fix.as_ref().unwrap().replacement;
        assert!(fixed.contains("## Contents"));
        assert!(fixed.contains("- Section 0"));
        // The contents list goes under the title rather than above it.
        assert!(fixed.starts_with("# Formats"));
    }

    #[test]
    fn a_long_file_that_already_has_a_contents_list_passes() {
        let mut text = String::from("# Formats\n\n## Contents\n\n- Section 0\n\n");
        for index in 0..40 {
            text.push_str(&format!("## Section {index}\n\nWords.\n\n"));
        }

        let mut skill = skill_with_body("\n## Culling\n\nRead references/formats.md.\n");
        skill
            .files
            .push(file("references/formats.md", &text, false));

        assert!(check(&CONTENTS_RULE, &skill).is_empty());
    }

    #[test]
    fn an_undeclared_import_is_reported() {
        let mut skill = skill_with_body("\n## Culling\n\nRun scripts/cull.py.\n");
        skill.files.push(file(
            "scripts/cull.py",
            "import rawpy\nimport os\n\nprint(rawpy)\n",
            true,
        ));

        let messages = check(&DEPENDENCIES_RULE, &skill);

        assert_eq!(messages.len(), 1, "the standard library does not count");
        assert!(messages[0].message.contains("rawpy"));
    }

    #[test]
    fn standard_library_modules_outside_the_old_allowlist_pass() {
        // Regression for https://github.com/MaximeGaudin/slint/issues/75: these ship with every
        // CPython install, so none of them needs an install instruction.
        for module in [
            "asyncio",
            "secrets",
            "threading",
            "multiprocessing",
            "socket",
            "queue",
            "decimal",
            "contextlib",
            "inspect",
            "pickle",
            "ssl",
            "platform",
            "tomllib",
            "ipaddress",
        ] {
            let mut skill = skill_with_body("\n## Culling\n\nRun scripts/fetch.py.\n");
            skill.files.push(file(
                "scripts/fetch.py",
                &format!("import {module}\n\nprint({module})\n"),
                true,
            ));

            assert!(check(&DEPENDENCIES_RULE, &skill).is_empty(), "for {module}");
        }
    }

    #[test]
    fn an_import_the_instructions_tell_you_to_install_passes() {
        let mut skill = skill_with_body(
            "\n## Culling\n\nFirst run `pip install rawpy`, then run scripts/cull.py.\n",
        );
        skill
            .files
            .push(file("scripts/cull.py", "import rawpy\n", true));

        assert!(check(&DEPENDENCIES_RULE, &skill).is_empty());
    }

    #[test]
    fn paths_are_found_in_the_shapes_instructions_actually_write_them() {
        let found =
            paths_in("Run `scripts/cull.py`, read references/formats.md, see (assets/logo.png).");

        assert!(found.contains("scripts/cull.py"));
        assert!(found.contains("references/formats.md"));
        assert!(found.contains("assets/logo.png"));
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/7 —
    /// bundled scripts need a Prerequisites (or equivalent) chapter so agents
    /// see host tools / network needs without reading the script itself.
    #[test]
    fn a_skill_with_scripts_and_no_prerequisites_section_is_reported() {
        let mut skill = skill_with_body(
            "\n## Script\n\nRun commands via `scripts/fetch.sh`.\n\n## Commands\n\n```bash\nscripts/fetch.sh get\n```\n",
        );
        skill.files.push(file(
            "scripts/fetch.sh",
            "#!/usr/bin/env bash\ncurl -sf https://example.com/api/demo | python3 -c 'print(1)'\n",
            true,
        ));

        let messages: Vec<_> = crate::engine::lint_skill(&skill, &crate::config::Config::default())
            .into_iter()
            .filter(|message| message.rule == "bundle/script-prerequisites")
            .collect();

        assert_eq!(
            messages.len(),
            1,
            "expected one prerequisites finding, got {messages:?}"
        );
        assert_eq!(messages[0].severity, Severity::Warning);
        assert!(
            messages[0]
                .message
                .to_ascii_lowercase()
                .contains("prerequisite")
                || messages[0]
                    .message
                    .to_ascii_lowercase()
                    .contains("requirement"),
            "expected the finding to name the missing section, got {}",
            messages[0].message
        );
    }

    #[test]
    fn a_prerequisites_section_satisfies_the_script_prerequisites_rule() {
        let mut skill = skill_with_body(
            "\n## Prerequisites\n\nRequires bash, curl, python3, and network access to example.com.\n\n## Script\n\nRun `scripts/fetch.sh`.\n",
        );
        skill.files.push(file(
            "scripts/fetch.sh",
            "#!/usr/bin/env bash\ncurl -sf https://example.com/api/demo\n",
            true,
        ));

        let messages: Vec<_> = crate::engine::lint_skill(&skill, &crate::config::Config::default())
            .into_iter()
            .filter(|message| message.rule == "bundle/script-prerequisites")
            .collect();

        assert!(
            messages.is_empty(),
            "expected Prerequisites to satisfy the rule, got {messages:?}"
        );
    }

    #[test]
    fn requirements_and_compatibility_headings_count_as_prerequisites() {
        for heading in ["## Requirements", "## Compatibility"] {
            let mut skill = skill_with_body(&format!(
                "\n{heading}\n\nNeeds curl.\n\n## Script\n\nRun `scripts/fetch.sh`.\n"
            ));
            skill
                .files
                .push(file("scripts/fetch.sh", "#!/usr/bin/env bash\n", true));

            let messages: Vec<_> =
                crate::engine::lint_skill(&skill, &crate::config::Config::default())
                    .into_iter()
                    .filter(|message| message.rule == "bundle/script-prerequisites")
                    .collect();

            assert!(
                messages.is_empty(),
                "expected {heading} to satisfy the rule, got {messages:?}"
            );
        }
    }

    #[test]
    fn a_skill_without_scripts_does_not_need_prerequisites() {
        let skill = skill_with_body("\n## Steps\n\n1. Import the files.\n");
        let messages: Vec<_> = crate::engine::lint_skill(&skill, &crate::config::Config::default())
            .into_iter()
            .filter(|message| message.rule == "bundle/script-prerequisites")
            .collect();

        assert!(messages.is_empty());
    }
}
