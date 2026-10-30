//! Release notes as lines to speak.

/// Markdown release notes as plain lines: no heading marks, bullets, link
/// targets, emphasis or rules, and no blank lines.
pub fn flatten_markdown(body: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.chars().all(|c| matches!(c, '-' | '=' | '*' | '_')) {
            continue;
        }
        let line = strip_heading(line);
        let line = strip_bullet(line);
        let line = strip_emphasis(&strip_links(line));
        let line = line.trim();
        if !line.is_empty() {
            lines.push(line.to_string());
        }
    }
    lines
}

fn strip_heading(line: &str) -> &str {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(char::is_whitespace) {
        line[hashes..].trim_start()
    } else {
        line
    }
}

fn strip_bullet(line: &str) -> &str {
    let mut chars = line.chars();
    match (chars.next(), chars.next()) {
        (Some('-' | '*' | '+'), Some(c)) if c.is_whitespace() => line[1..].trim_start(),
        _ => line,
    }
}

/// `[text](target)` becomes `text`.
fn strip_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find("](").and_then(|close| {
            let target = &after[close + 2..];
            target.find(')').map(|end| (close, close + 2 + end + 1))
        });
        match link {
            Some((close, end)) if !after[..close].contains('[') => {
                out.push_str(&rest[..open]);
                out.push_str(&after[..close]);
                rest = &after[end..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_emphasis(line: &str) -> String {
    line.chars()
        .filter(|c| !matches!(c, '*' | '_' | '`'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_becomes_plain_lines() {
        let notes =
            "## What's new\n\n- **Faster** menus\n* See [the guide](https://example.com/guide).\n\
                     ---\n1. Numbered stays\n`code` and _words_\n#hashtag stays\n";
        assert_eq!(
            flatten_markdown(notes),
            [
                "What's new",
                "Faster menus",
                "See the guide.",
                "1. Numbered stays",
                "code and words",
                "#hashtag stays",
            ]
        );
    }

    #[test]
    fn brackets_that_are_not_links_stay() {
        assert_eq!(flatten_markdown("[beta] only"), ["[beta] only"]);
        assert_eq!(flatten_markdown("a [b [c](d) e"), ["a [b c e"]);
        assert!(flatten_markdown("").is_empty());
    }
}
