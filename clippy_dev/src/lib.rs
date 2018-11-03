// Copyright 2014-2018 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.


#![allow(clippy::default_hash_types)]

use itertools::Itertools;
use lazy_static::lazy_static;
use regex::Regex;
use walkdir::WalkDir;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::prelude::*;

lazy_static! {
    static ref DEC_CLIPPY_LINT_RE: Regex = Regex::new(r#"(?x)
        declare_clippy_lint!\s*[\{(]\s*
        pub\s+(?P<name>[A-Z_][A-Z_0-9]*)\s*,\s*
        (?P<cat>[a-z_]+)\s*,\s*
        "(?P<desc>(?:[^"\\]+|\\(?s).(?-s))*)"\s*[})]
    "#).unwrap();
    static ref DEC_DEPRECATED_LINT_RE: Regex = Regex::new(r#"(?x)
        declare_deprecated_lint!\s*[{(]\s*
        pub\s+(?P<name>[A-Z_][A-Z_0-9]*)\s*,\s*
        "(?P<desc>(?:[^"\\]+|\\(?s).(?-s))*)"\s*[})]
    "#).unwrap();
    static ref NL_ESCAPE_RE: Regex = Regex::new(r#"\\\n\s*"#).unwrap();
    pub static ref DOCS_LINK: String = "https://rust-lang-nursery.github.io/rust-clippy/master/index.html".to_string();
}

/// Lint data parsed from the Clippy source code.
#[derive(Clone, PartialEq, Debug)]
pub struct Lint {
    pub name: String,
    pub group: String,
    pub desc: String,
    pub deprecation: Option<String>,
    pub module: String,
}

impl Lint {
    pub fn new(name: &str, group: &str, desc: &str, deprecation: Option<&str>, module: &str) -> Self {
        Self {
            name: name.to_lowercase(),
            group: group.to_string(),
            desc: NL_ESCAPE_RE.replace(&desc.replace("\\\"", "\""), "").to_string(),
            deprecation: deprecation.map(|d| d.to_string()),
            module: module.to_string(),
        }
    }

    /// Returns all non-deprecated lints and non-internal lints
    pub fn usable_lints(lints: impl Iterator<Item=Self>) -> impl Iterator<Item=Self> {
        lints.filter(|l| l.deprecation.is_none() && !l.is_internal())
    }

    /// Returns the lints in a HashMap, grouped by the different lint groups
    pub fn by_lint_group(lints: &[Self]) -> HashMap<String, Vec<Self>> {
        lints.iter().map(|lint| (lint.group.to_string(), lint.clone())).into_group_map()
    }

    pub fn is_internal(&self) -> bool {
        self.group.starts_with("internal")
    }
}

/// Generates the list of lint links at the bottom of the README
pub fn gen_changelog_lint_list(lints: Vec<Lint>) -> Vec<String> {
    let mut lint_list_sorted: Vec<Lint> = lints;
    lint_list_sorted.sort_by_key(|l| l.name.clone());
    lint_list_sorted
        .iter()
        .filter_map(|l| {
            if l.is_internal() {
                None
            } else {
                Some(format!("[`{}`]: {}#{}", l.name, DOCS_LINK.clone(), l.name))
            }
        }).collect()
}

/// Generates the `register_removed` code in `./clippy_lints/src/lib.rs`.
pub fn gen_deprecated(lints: &[Lint]) -> Vec<String> {
    lints.iter()
        .filter_map(|l| {
            l.clone().deprecation.and_then(|depr_text| {
                Some(
                    format!(
                        "    store.register_removed(\n        \"{}\",\n        \"{}\",\n    );",
                        l.name,
                        depr_text
                    )
                )
            })
        })
        .collect()
}

/// Gathers all files in `src/clippy_lints` and gathers all lints inside
pub fn gather_all() -> impl Iterator<Item=Lint> {
    lint_files().flat_map(|f| gather_from_file(&f))
}

fn gather_from_file(dir_entry: &walkdir::DirEntry) -> impl Iterator<Item=Lint> {
    let mut file = fs::File::open(dir_entry.path()).unwrap();
    let mut content = String::new();
    file.read_to_string(&mut content).unwrap();
    parse_contents(&content, dir_entry.path().file_stem().unwrap().to_str().unwrap())
}

fn parse_contents(content: &str, filename: &str) -> impl Iterator<Item=Lint> {
    let lints = DEC_CLIPPY_LINT_RE
        .captures_iter(content)
        .map(|m| Lint::new(&m["name"], &m["cat"], &m["desc"], None, filename));
    let deprecated = DEC_DEPRECATED_LINT_RE
        .captures_iter(content)
        .map(|m| Lint::new( &m["name"], "Deprecated", &m["desc"], Some(&m["desc"]), filename));
    // Removing the `.collect::<Vec<Lint>>().into_iter()` causes some lifetime issues due to the map
    lints.chain(deprecated).collect::<Vec<Lint>>().into_iter()
}

/// Collects all .rs files in the `clippy_lints/src` directory
fn lint_files() -> impl Iterator<Item=walkdir::DirEntry> {
    // We use `WalkDir` instead of `fs::read_dir` here in order to recurse into subdirectories.
    // Otherwise we would not collect all the lints, for example in `clippy_lints/src/methods/`.
    WalkDir::new("../clippy_lints/src")
        .into_iter()
        .filter_map(|f| f.ok())
        .filter(|f| f.path().extension() == Some(OsStr::new("rs")))
}

/// Replace a region in a file delimited by two lines matching regexes.
///
/// `path` is the relative path to the file on which you want to perform the replacement.
///
/// See `replace_region_in_text` for documentation of the other options.
#[allow(clippy::expect_fun_call)]
pub fn replace_region_in_file<F>(path: &str, start: &str, end: &str, replace_start: bool, replacements: F) where F: Fn() -> Vec<String> {
    let mut f = fs::File::open(path).expect(&format!("File not found: {}", path));
    let mut contents = String::new();
    f.read_to_string(&mut contents).expect("Something went wrong reading the file");
    let replaced = replace_region_in_text(&contents, start, end, replace_start, replacements);

    let mut f = fs::File::create(path).expect(&format!("File not found: {}", path));
    f.write_all(replaced.as_bytes()).expect("Unable to write file");
    // Ensure we write the changes with a trailing newline so that
    // the file has the proper line endings.
    f.write_all(b"\n").expect("Unable to write file");
}

/// Replace a region in a text delimited by two lines matching regexes.
///
/// * `text` is the input text on which you want to perform the replacement
/// * `start` is a `&str` that describes the delimiter line before the region you want to replace.
///   As the `&str` will be converted to a `Regex`, this can contain regex syntax, too.
/// * `end` is a `&str` that describes the delimiter line until where the replacement should
///   happen.  As the `&str` will be converted to a `Regex`, this can contain regex syntax, too.
/// * If `replace_start` is true, the `start` delimiter line is replaced as well.
///   The `end` delimiter line is never replaced.
/// * `replacements` is a closure that has to return a `Vec<String>` which contains the new text.
///
/// If you want to perform the replacement on files instead of already parsed text,
/// use `replace_region_in_file`.
///
/// # Example
///
/// ```
/// let the_text = "replace_start\nsome text\nthat will be replaced\nreplace_end";
/// let result = clippy_dev::replace_region_in_text(
///     the_text,
///     r#"replace_start"#,
///     r#"replace_end"#,
///     false,
///     || {
///         vec!["a different".to_string(), "text".to_string()]
///     }
/// );
/// assert_eq!("replace_start\na different\ntext\nreplace_end", result);
/// ```
pub fn replace_region_in_text<F>(text: &str, start: &str, end: &str, replace_start: bool, replacements: F) -> String where F: Fn() -> Vec<String> {
    let lines = text.lines();
    let mut in_old_region = false;
    let mut found = false;
    let mut new_lines = vec![];
    let start = Regex::new(start).unwrap();
    let end = Regex::new(end).unwrap();

    for line in lines {
        if in_old_region {
            if end.is_match(&line) {
                in_old_region = false;
                new_lines.extend(replacements());
                new_lines.push(line.to_string());
            }
        } else if start.is_match(&line) {
            if !replace_start {
                new_lines.push(line.to_string());
            }
            in_old_region = true;
            found = true;
        } else {
            new_lines.push(line.to_string());
        }
    }

    if !found {
        // This happens if the provided regex in `clippy_dev/src/main.rs` is not found in the
        // given text or file. Most likely this is an error on the programmer's side and the Regex
        // is incorrect.
        println!("regex {:?} not found. You may have to update it.", start);
    }
    new_lines.join("\n")
}

#[test]
fn test_parse_contents() {
    let result: Vec<Lint> = parse_contents(
        r#"
declare_clippy_lint! {
    pub PTR_ARG,
    style,
    "really long \
     text"
}

declare_clippy_lint!{
    pub DOC_MARKDOWN,
    pedantic,
    "single line"
}

/// some doc comment
declare_deprecated_lint! {
    pub SHOULD_ASSERT_EQ,
    "`assert!()` will be more flexible with RFC 2011"
}
    "#,
    "module_name").collect();

    let expected = vec![
        Lint::new("ptr_arg", "style", "really long text", None, "module_name"),
        Lint::new("doc_markdown", "pedantic", "single line", None, "module_name"),
        Lint::new(
            "should_assert_eq",
            "Deprecated",
            "`assert!()` will be more flexible with RFC 2011",
            Some("`assert!()` will be more flexible with RFC 2011"),
            "module_name"
        ),
    ];
    assert_eq!(expected, result);
}

#[test]
fn test_replace_region() {
    let text = r#"
abc
123
789
def
ghi"#;
    let expected = r#"
abc
hello world
def
ghi"#;
    let result = replace_region_in_text(text, r#"^\s*abc$"#, r#"^\s*def"#, false, || {
        vec!["hello world".to_string()]
    });
    assert_eq!(expected, result);
}

#[test]
fn test_replace_region_with_start() {
    let text = r#"
abc
123
789
def
ghi"#;
    let expected = r#"
hello world
def
ghi"#;
    let result = replace_region_in_text(text, r#"^\s*abc$"#, r#"^\s*def"#, true, || {
        vec!["hello world".to_string()]
    });
    assert_eq!(expected, result);
}

#[test]
fn test_usable_lints() {
    let lints = vec![
        Lint::new("should_assert_eq", "Deprecated", "abc", Some("Reason"), "module_name"),
        Lint::new("should_assert_eq2", "Not Deprecated", "abc", None, "module_name"),
        Lint::new("should_assert_eq2", "internal", "abc", None, "module_name"),
        Lint::new("should_assert_eq2", "internal_style", "abc", None, "module_name")
    ];
    let expected = vec![
        Lint::new("should_assert_eq2", "Not Deprecated", "abc", None, "module_name")
    ];
    assert_eq!(expected, Lint::usable_lints(lints.into_iter()).collect::<Vec<Lint>>());
}

#[test]
fn test_by_lint_group() {
    let lints = vec![
        Lint::new("should_assert_eq", "group1", "abc", None, "module_name"),
        Lint::new("should_assert_eq2", "group2", "abc", None, "module_name"),
        Lint::new("incorrect_match", "group1", "abc", None, "module_name"),
    ];
    let mut expected: HashMap<String, Vec<Lint>> = HashMap::new();
    expected.insert("group1".to_string(), vec![
        Lint::new("should_assert_eq", "group1", "abc", None, "module_name"),
        Lint::new("incorrect_match", "group1", "abc", None, "module_name"),
    ]);
    expected.insert("group2".to_string(), vec![
        Lint::new("should_assert_eq2", "group2", "abc", None, "module_name")
    ]);
    assert_eq!(expected, Lint::by_lint_group(&lints));
}

#[test]
fn test_gen_changelog_lint_list() {
    let lints = vec![
        Lint::new("should_assert_eq", "group1", "abc", None, "module_name"),
        Lint::new("should_assert_eq2", "group2", "abc", None, "module_name"),
        Lint::new("incorrect_internal", "internal_style", "abc", None, "module_name"),
    ];
    let expected = vec![
        format!("[`should_assert_eq`]: {}#should_assert_eq", DOCS_LINK.to_string()),
        format!("[`should_assert_eq2`]: {}#should_assert_eq2", DOCS_LINK.to_string())
    ];
    assert_eq!(expected, gen_changelog_lint_list(lints));
}

#[test]
fn test_gen_deprecated() {
    let lints = vec![
        Lint::new("should_assert_eq", "group1", "abc", Some("has been superseeded by should_assert_eq2"), "module_name"),
        Lint::new("should_assert_eq2", "group2", "abc", None, "module_name")
    ];
    let expected: Vec<String> = vec![
        r#"    store.register_removed(
        "should_assert_eq",
        "has been superseeded by should_assert_eq2",
    );"#.to_string()
    ];
    assert_eq!(expected, gen_deprecated(&lints));
}