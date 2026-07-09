use crate::types::file_result::AiFileOperationStats;

pub fn diff_stats(before: &str, after: &str, created: bool) -> AiFileOperationStats {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let common = lcs_len(&before_lines, &after_lines);
    AiFileOperationStats {
        lines_added: after_lines.len() - common,
        lines_removed: before_lines.len() - common,
        files_changed: usize::from(!created && before != after),
        files_created: usize::from(created),
        files_deleted: 0,
    }
}

pub fn combine_patch_stats(stats: &[AiFileOperationStats]) -> AiFileOperationStats {
    stats.iter().fold(
        AiFileOperationStats::default(),
        |mut acc, s| {
            acc.lines_added += s.lines_added;
            acc.lines_removed += s.lines_removed;
            acc.files_changed += s.files_changed;
            acc.files_created += s.files_created;
            acc.files_deleted += s.files_deleted;
            acc
        },
    )
}

fn lcs_len(a: &[&str], b: &[&str]) -> usize {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if short.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; short.len() + 1];
    let mut curr = vec![0usize; short.len() + 1];
    for &long_line in long {
        for (j, &short_line) in short.iter().enumerate() {
            curr[j + 1] = if long_line == short_line {
                prev[j] + 1
            } else {
                curr[j].max(prev[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[short.len()]
}
