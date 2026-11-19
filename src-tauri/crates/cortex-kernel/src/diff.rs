//! A line diff, for reviewing what changed between two versions of something the assistant wrote.

/// One line of a diff: kept, removed from the old text, or added in the new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Same(String),
    Removed(String),
    Added(String),
}

/// The lines that turn `old` into `new`, by longest common subsequence.
pub fn lines(old: &str, new: &str) -> Vec<Line> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(Line::Same(a[i].to_string()));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(Line::Removed(a[i].to_string()));
            i += 1;
        } else {
            out.push(Line::Added(b[j].to_string()));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|l| Line::Removed(l.to_string())));
    out.extend(b[j..].iter().map(|l| Line::Added(l.to_string())));
    out
}

/// The diff as text, `-` and `+` before changed lines, only the changes and one line around each.
pub fn unified(old: &str, new: &str) -> String {
    let diff = lines(old, new);
    let changed = |k: usize| diff.get(k).is_some_and(|l| !matches!(l, Line::Same(_)));
    let mut out = String::new();
    for (k, line) in diff.iter().enumerate() {
        let near = changed(k) || changed(k + 1) || (k > 0 && changed(k - 1));
        if !near {
            continue;
        }
        match line {
            Line::Same(l) => out.push_str(&format!("  {l}\n")),
            Line::Removed(l) => out.push_str(&format!("- {l}\n")),
            Line::Added(l) => out.push_str(&format!("+ {l}\n")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_found_line_by_line() {
        let d = lines("a\nb\nc", "a\nB\nc\nd");
        assert_eq!(
            d,
            vec![
                Line::Same("a".into()),
                Line::Removed("b".into()),
                Line::Added("B".into()),
                Line::Same("c".into()),
                Line::Added("d".into()),
            ]
        );
        assert_eq!(unified("x\ny", "x\ny"), "");
        assert!(unified("1\n2\n3\n4\n5", "1\n2\nthree\n4\n5").contains("- 3\n+ three"));
    }
}
