//! Diff tối giản cho preview trước khi ghi file.
//!
//! Không phụ thuộc crate ngoài: LCS O(n*m) có giới hạn kích thước, đủ cho file
//! nguồn thông thường. File quá lớn rơi về diff thay toàn bộ.

const CONTEXT_LINES: usize = 3;
const MAX_CELLS: usize = 1_000_000;

#[derive(Debug, Clone, Copy)]
enum Op {
    Equal(usize),
    Delete(usize),
    Insert(usize),
}

/// Unified diff giữa `old` và `new`, dạng `--- a/path` / `+++ b/path`.
/// Trả chuỗi rỗng khi hai nội dung giống nhau.
pub fn unified_diff(old: &str, new: &str, path: &str) -> String {
    if old == new {
        return String::new();
    }

    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let ops = diff_ops(&old_lines, &new_lines);
    render(&ops, &old_lines, &new_lines, path)
}

fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn diff_ops(old: &[&str], new: &[&str]) -> Vec<Op> {
    let rows = old.len();
    let cols = new.len();
    if rows.saturating_mul(cols) > MAX_CELLS {
        let mut ops = Vec::with_capacity(rows + cols);
        ops.extend((0..rows).map(Op::Delete));
        ops.extend((0..cols).map(Op::Insert));
        return ops;
    }

    // dp[i][j] = độ dài LCS của old[i..] và new[j..].
    let width = cols + 1;
    let mut dp = vec![0u32; (rows + 1) * width];
    for i in (0..rows).rev() {
        for j in (0..cols).rev() {
            dp[i * width + j] = if old[i] == new[j] {
                dp[(i + 1) * width + j + 1] + 1
            } else {
                dp[(i + 1) * width + j].max(dp[i * width + j + 1])
            };
        }
    }

    let mut ops = Vec::with_capacity(rows + cols);
    let (mut i, mut j) = (0, 0);
    while i < rows && j < cols {
        if old[i] == new[j] {
            ops.push(Op::Equal(i));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * width + j] >= dp[i * width + j + 1] {
            ops.push(Op::Delete(i));
            i += 1;
        } else {
            ops.push(Op::Insert(j));
            j += 1;
        }
    }
    while i < rows {
        ops.push(Op::Delete(i));
        i += 1;
    }
    while j < cols {
        ops.push(Op::Insert(j));
        j += 1;
    }
    ops
}

fn render(ops: &[Op], old: &[&str], new: &[&str], path: &str) -> String {
    let mut changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter_map(|(index, op)| (!matches!(op, Op::Equal(_))).then_some(index))
        .collect();
    if changed.is_empty() {
        return String::new();
    }

    // Mở rộng mỗi thay đổi thêm CONTEXT_LINES và gộp các khoảng chồng nhau.
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for index in changed.drain(..) {
        let start = index.saturating_sub(CONTEXT_LINES);
        let end = (index + CONTEXT_LINES + 1).min(ops.len());
        match groups.last_mut() {
            Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
            _ => groups.push((start, end)),
        }
    }

    let mut old_before = vec![0usize; ops.len() + 1];
    let mut new_before = vec![0usize; ops.len() + 1];
    for (index, op) in ops.iter().enumerate() {
        old_before[index + 1] = old_before[index] + usize::from(!matches!(op, Op::Insert(_)));
        new_before[index + 1] = new_before[index] + usize::from(!matches!(op, Op::Delete(_)));
    }

    let mut out = String::new();
    out.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
    for (start, end) in groups {
        let old_count = old_before[end] - old_before[start];
        let new_count = new_before[end] - new_before[start];
        let old_start = if old_count == 0 {
            old_before[start]
        } else {
            old_before[start] + 1
        };
        let new_start = if new_count == 0 {
            new_before[start]
        } else {
            new_before[start] + 1
        };
        out.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));
        for op in &ops[start..end] {
            match *op {
                Op::Equal(old_index) => {
                    out.push(' ');
                    out.push_str(old[old_index]);
                }
                Op::Delete(old_index) => {
                    out.push('-');
                    out.push_str(old[old_index]);
                }
                Op::Insert(new_index) => {
                    out.push('+');
                    out.push_str(new[new_index]);
                }
            }
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_has_no_diff() {
        assert!(unified_diff("a\nb\n", "a\nb\n", "src/a.rs").is_empty());
    }

    #[test]
    fn shows_changed_line() {
        let diff = unified_diff("a\nb\nc\n", "a\nB\nc\n", "src/a.rs");
        assert!(diff.starts_with("--- a/src/a.rs\n+++ b/src/a.rs\n"));
        assert!(diff.contains("-b\n+B\n"));
        assert!(diff.contains(" a\n"));
        assert!(diff.contains(" c\n"));
    }

    #[test]
    fn shows_insert_and_delete() {
        let diff = unified_diff("one\ntwo\n", "one\nnew\ntwo\nthree\n", "notes.md");
        assert!(diff.contains("+new\n"));
        assert!(diff.contains("+three\n"));
        assert!(!diff.contains("-one"));
    }

    #[test]
    fn new_file_starts_at_zero() {
        let diff = unified_diff("", "hello\n", "README.md");
        assert!(diff.contains("@@ -0,0 +1,1 @@"));
        assert!(diff.contains("+hello\n"));
    }
}
