//! Lightweight checks a game's CI can run on `lang/*.po`.

use std::path::Path;

use crate::l10n::{matcher, po, Catalog};

/// One problem found in a catalog file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckIssue {
    /// Language code (file stem), or `"*"` for a directory-level note.
    pub code: String,
    /// Human-readable problem.
    pub message: String,
}

/// Check one PO file's text: parse errors, unknown placeholders, and
/// compile problems recorded by [`Catalog::from_po`].
pub fn check_catalog(code: &str, text: &str) -> Vec<CheckIssue> {
    let mut issues = Vec::new();
    let entries = match po::parse(text) {
        Ok(entries) => entries,
        Err(err) => {
            issues.push(CheckIssue {
                code: code.to_string(),
                message: err.to_string(),
            });
            return issues;
        }
    };
    for entry in &entries {
        if entry.msgid.is_empty() || entry.msgstr.is_empty() || entry.is_fuzzy() {
            continue;
        }
        let segs = matcher::parse_source(&entry.msgid);
        let names = matcher::hole_names(&segs);
        let translation = matcher::parse_translation(&entry.msgstr);
        let unknown: Vec<&str> = translation
            .iter()
            .filter_map(|part| match part {
                matcher::Part::Hole(name) | matcher::Part::Plural(name, _) => Some(name.as_str()),
                matcher::Part::Lit(_) => None,
            })
            .filter(|name| !names.iter().any(|n| n == name))
            .collect();
        if !unknown.is_empty() {
            issues.push(CheckIssue {
                code: code.to_string(),
                message: format!(
                    "line {}: translation names {{{}}}, which the English line does not have",
                    entry.line,
                    unknown.join("}, {")
                ),
            });
        }
    }
    if let Ok(catalog) = Catalog::from_po(code, text) {
        for problem in catalog.problems {
            issues.push(CheckIssue {
                code: code.to_string(),
                message: problem,
            });
        }
    }
    issues
}

/// Check every `*.po` in `dir`. Missing directory yields no issues.
pub fn check_dir(dir: &Path) -> Vec<CheckIssue> {
    let mut issues = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return issues;
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "po"))
        .collect();
    paths.sort();
    for path in paths {
        let Some(code) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => issues.extend(check_catalog(code, &text)),
            Err(err) => issues.push(CheckIssue {
                code: code.to_string(),
                message: format!("could not read {}: {err}", path.display()),
            }),
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_placeholder_is_reported() {
        let issues = check_catalog(
            "xx",
            "msgid \"Hello {name}.\"\nmsgstr \"Hola {nombre}.\"\n",
        );
        assert!(
            issues.iter().any(|i| i.message.contains("nombre")),
            "{issues:?}"
        );
    }

    #[test]
    fn clean_catalog_has_no_issues() {
        let issues = check_catalog("xx", "msgid \"Hello {name}.\"\nmsgstr \"Hola {name}.\"\n");
        assert!(issues.is_empty(), "{issues:?}");
    }
}
