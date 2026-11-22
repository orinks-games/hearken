//! Small text and file helpers the extractor and checker share.

use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;

pub fn rel(path: &Path, base: &Path) -> String {
    let shown = path.strip_prefix(base).unwrap_or(path);
    shown
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn py_isspace(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}

pub fn py_strip(s: &str) -> &str {
    s.trim_matches(py_isspace)
}

pub fn py_lstrip(s: &str) -> &str {
    s.trim_start_matches(py_isspace)
}

pub fn py_rstrip(s: &str) -> &str {
    s.trim_end_matches(py_isspace)
}

pub fn py_splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes: Vec<(usize, char)> = s.char_indices().collect();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let (pos, c) = bytes[i];
        let is_break = matches!(
            c,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if is_break {
            out.push(&s[start..pos]);
            let mut next = pos + c.len_utf8();
            if c == '\r' && i + 1 < bytes.len() && bytes[i + 1].1 == '\n' {
                i += 1;
                next += 1;
            }
            start = next;
        }
        i += 1;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

pub fn sort_paths(paths: &mut [PathBuf]) {
    paths.sort_by(|a, b| {
        let ca: Vec<String> = a
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let cb: Vec<String> = b
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        ca.cmp(&cb)
    });
}

pub fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => out.push(path),
                Ok(t) if t.is_symlink() && path.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    sort_paths(&mut out);
    out
}

pub fn glob_suffix(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(suffix))
        })
        .collect();
    sort_paths(&mut out);
    out
}

pub fn read_text(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(if text.contains('\r') {
        text.replace("\r\n", "\n")
    } else {
        text
    })
}

pub fn read_text_lf(path: &Path) -> Option<String> {
    read_text(path).ok()
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn py_round0(x: f64) -> String {
    let r = x.round_ties_even();
    format!("{}", r as i64)
}
