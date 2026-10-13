//! Repository policy: code, comments and documents are English. Text in other languages
//! lives only in locale and model-text resource directories (see `AGENTS.md`).
//!
//! The check is mechanical: outside those directories a file may contain ASCII and a few
//! typographic characters, nothing else. Tests that need other scripts write them as
//! `\u{...}` escapes.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Directories whose files may contain any text.
const MULTILINGUAL_DIRS: [&str; 3] = [
    "crates/nekowriter-agent/resources/",
    "crates/nekowriter-app/resources/",
    "webui/src/locales/zh-CN/",
];

/// Typographic characters allowed everywhere: em dash, en dash, ellipsis, curly quotes,
/// middle dot, arrow, multiplication sign and section sign.
const TYPOGRAPHY: [char; 11] = [
    '\u{2014}', '\u{2013}', '\u{2026}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{b7}',
    '\u{2192}', '\u{d7}', '\u{a7}',
];

/// Extensions of binary files, which are not text and are not checked.
const BINARY_EXTENSIONS: [&str; 10] = [
    "png", "jpg", "jpeg", "gif", "webp", "ico", "woff", "woff2", "ttf", "otf",
];

#[test]
fn files_outside_resource_directories_are_english() {
    let root = repository_root();
    let mut violations = Vec::new();
    for path in tracked_files(&root) {
        let relative = path.to_string_lossy().replace('\\', "/");
        if MULTILINGUAL_DIRS
            .iter()
            .any(|dir| relative.starts_with(dir))
            || is_binary(&path)
        {
            continue;
        }
        let Ok(bytes) = fs::read(root.join(&path)) else {
            // Listed by git but deleted in the working tree.
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            violations.push(format!("{relative}: not UTF-8 text"));
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            for (column, character) in line.chars().enumerate() {
                if !character.is_ascii() && !TYPOGRAPHY.contains(&character) {
                    violations.push(format!(
                        "{relative}:{}:{}: U+{:04X}",
                        line_index + 1,
                        column + 1,
                        u32::from(character)
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "non-English text outside the resource directories (move it into a resource file, \
         or write it as a \\u{{...}} escape in a test):\n{}",
        violations.join("\n")
    );
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Tracked files plus new files that are not ignored, so a file is checked before its
/// first commit.
fn tracked_files(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()
        .expect("git runs (this test lists the repository's files with git)");
    assert!(output.status.success(), "git ls-files failed");
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(String::from_utf8_lossy(path).into_owned()))
        .collect()
}

fn is_binary(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            BINARY_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}
