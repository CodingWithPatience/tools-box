use similar::{ChangeTag, TextDiff};

use super::models::{DiffHunk, DiffLine, DiffResult, DiffType, SplitLine, TextSegment};

/// 计算两段文本的差异
pub fn compute_diff(left: &str, right: &str) -> DiffResult {
    let diff = TextDiff::from_lines(left, right);

    let unified_lines = build_unified_lines(&diff);
    let split_lines = build_split_lines(&diff);
    let unified_diff_hunks = build_unified_diff_hunks(&unified_lines);
    let diff_hunks = build_diff_hunks(&split_lines);

    let mut added_count = 0;
    let mut removed_count = 0;

    for line in &unified_lines {
        match line.diff_type {
            DiffType::Added => added_count += 1,
            DiffType::Removed => removed_count += 1,
            DiffType::Equal => {}
        }
    }

    // 计算相似度
    let similarity: f64 = diff.ratio().into();

    DiffResult {
        unified_lines,
        unified_diff_hunks,
        split_lines,
        diff_hunks,
        added_count,
        removed_count,
        similarity,
    }
}

/// 将 Unified 视图中的连续差异行合并为差异块
fn build_unified_diff_hunks(lines: &[DiffLine]) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut start_line = None;

    for (line_index, line) in lines.iter().enumerate() {
        if line.diff_type != DiffType::Equal {
            if start_line.is_none() {
                start_line = Some(line_index);
            }
        } else if let Some(start) = start_line.take() {
            hunks.push(DiffHunk {
                start_line: start,
                end_line: line_index - 1,
            });
        }
    }

    if let Some(start) = start_line {
        hunks.push(DiffHunk {
            start_line: start,
            end_line: lines.len() - 1,
        });
    }

    hunks
}

/// 将 Split 视图中的连续差异行合并为差异块
fn build_diff_hunks(lines: &[SplitLine]) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut start_line = None;

    for (line_index, line) in lines.iter().enumerate() {
        let is_changed = line.left_type != DiffType::Equal || line.right_type != DiffType::Equal;

        if is_changed {
            if start_line.is_none() {
                start_line = Some(line_index);
            }
        } else if let Some(start) = start_line.take() {
            hunks.push(DiffHunk {
                start_line: start,
                end_line: line_index - 1,
            });
        }
    }

    if let Some(start) = start_line {
        hunks.push(DiffHunk {
            start_line: start,
            end_line: lines.len() - 1,
        });
    }

    hunks
}

/// 计算两行文本的字符级差异
fn compute_char_diff(old: &str, new: &str) -> (Vec<TextSegment>, Vec<TextSegment>) {
    let diff = TextDiff::from_chars(old, new);

    let mut old_segments = Vec::new();
    let mut new_segments = Vec::new();

    for change in diff.iter_all_changes() {
        let value = change.value().to_string();
        match change.tag() {
            ChangeTag::Equal => {
                old_segments.push(TextSegment {
                    text: value.clone(),
                    diff_type: DiffType::Equal,
                });
                new_segments.push(TextSegment {
                    text: value,
                    diff_type: DiffType::Equal,
                });
            }
            ChangeTag::Delete => {
                old_segments.push(TextSegment {
                    text: value,
                    diff_type: DiffType::Removed,
                });
            }
            ChangeTag::Insert => {
                new_segments.push(TextSegment {
                    text: value,
                    diff_type: DiffType::Added,
                });
            }
        }
    }

    (old_segments, new_segments)
}

/// 构建 Unified 视图数据
fn build_unified_lines<'a>(diff: &TextDiff<'a, 'a, '_, str>) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let mut left_line = 1;
    let mut right_line = 1;

    // 收集连续的删除和插入操作，用于配对显示字符级差异
    let changes: Vec<_> = diff.iter_all_changes().collect();
    let mut i = 0;

    while i < changes.len() {
        let change = &changes[i];
        let content = change.value().trim_end_matches('\n').to_string();

        match change.tag() {
            ChangeTag::Equal => {
                let segments = vec![TextSegment {
                    text: content.clone(),
                    diff_type: DiffType::Equal,
                }];
                lines.push(DiffLine {
                    line_number_left: Some(left_line),
                    line_number_right: Some(right_line),
                    content,
                    diff_type: DiffType::Equal,
                    segments,
                });
                left_line += 1;
                right_line += 1;
                i += 1;
            }
            ChangeTag::Delete => {
                // 检查后面是否有对应的插入操作（修改操作）
                let mut deletes = Vec::new();
                let mut inserts = Vec::new();

                // 收集连续的删除操作
                while i < changes.len() && changes[i].tag() == ChangeTag::Delete {
                    let c = changes[i].value().trim_end_matches('\n').to_string();
                    deletes.push(c);
                    i += 1;
                }

                // 收集连续的插入操作
                while i < changes.len() && changes[i].tag() == ChangeTag::Insert {
                    let c = changes[i].value().trim_end_matches('\n').to_string();
                    inserts.push(c);
                    i += 1;
                }

                // 配对删除和插入操作，多出的行单独作为纯删除/纯新增处理
                let max_count = deletes.len().max(inserts.len());
                for j in 0..max_count {
                    if j < deletes.len() {
                        let left_content = &deletes[j];
                        // 如果有对应的插入行，计算字符级差异并缓存结果
                        let segments = if j < inserts.len() {
                            let (old_segments, _) = compute_char_diff(left_content, &inserts[j]);
                            old_segments
                        } else {
                            vec![TextSegment {
                                text: left_content.clone(),
                                diff_type: DiffType::Removed,
                            }]
                        };
                        lines.push(DiffLine {
                            line_number_left: Some(left_line),
                            line_number_right: None,
                            content: left_content.clone(),
                            diff_type: DiffType::Removed,
                            segments,
                        });
                        left_line += 1;
                    }

                    if j < inserts.len() {
                        let right_content = &inserts[j];
                        // 如果有对应的删除行，复用之前的计算结果
                        let segments = if j < deletes.len() {
                            let (_, new_segments) = compute_char_diff(&deletes[j], right_content);
                            new_segments
                        } else {
                            vec![TextSegment {
                                text: right_content.clone(),
                                diff_type: DiffType::Added,
                            }]
                        };
                        lines.push(DiffLine {
                            line_number_left: None,
                            line_number_right: Some(right_line),
                            content: right_content.clone(),
                            diff_type: DiffType::Added,
                            segments,
                        });
                        right_line += 1;
                    }
                }
            }
            ChangeTag::Insert => {
                // 纯插入（没有对应的删除）
                let segments = vec![TextSegment {
                    text: content.clone(),
                    diff_type: DiffType::Added,
                }];
                lines.push(DiffLine {
                    line_number_left: None,
                    line_number_right: Some(right_line),
                    content,
                    diff_type: DiffType::Added,
                    segments,
                });
                right_line += 1;
                i += 1;
            }
        }
    }

    lines
}

/// 构建 Split 视图数据
fn build_split_lines<'a>(diff: &TextDiff<'a, 'a, '_, str>) -> Vec<SplitLine> {
    let mut lines = Vec::new();
    let mut left_line = 1;
    let mut right_line = 1;

    // 收集连续的删除和插入操作，用于配对显示
    let changes: Vec<_> = diff.iter_all_changes().collect();
    let mut i = 0;

    while i < changes.len() {
        let change = &changes[i];
        let content = change.value().trim_end_matches('\n').to_string();

        match change.tag() {
            ChangeTag::Equal => {
                lines.push(SplitLine {
                    left_line_number: Some(left_line),
                    left_content: Some(content.clone()),
                    left_type: DiffType::Equal,
                    left_segments: vec![TextSegment {
                        text: content.clone(),
                        diff_type: DiffType::Equal,
                    }],
                    right_line_number: Some(right_line),
                    right_content: Some(content.clone()),
                    right_type: DiffType::Equal,
                    right_segments: vec![TextSegment {
                        text: content,
                        diff_type: DiffType::Equal,
                    }],
                });
                left_line += 1;
                right_line += 1;
                i += 1;
            }
            ChangeTag::Delete => {
                // 检查后面是否有对应的插入操作（修改操作）
                let mut deletes = Vec::new();
                let mut inserts = Vec::new();

                // 收集连续的删除操作
                while i < changes.len() && changes[i].tag() == ChangeTag::Delete {
                    let c = changes[i].value().trim_end_matches('\n').to_string();
                    deletes.push(c);
                    i += 1;
                }

                // 收集连续的插入操作
                while i < changes.len() && changes[i].tag() == ChangeTag::Insert {
                    let c = changes[i].value().trim_end_matches('\n').to_string();
                    inserts.push(c);
                    i += 1;
                }

                // 配对删除和插入操作
                let max_count = deletes.len().max(inserts.len());
                for j in 0..max_count {
                    let left_data = if j < deletes.len() {
                        left_line += 1;
                        Some((left_line - 1, deletes[j].clone()))
                    } else {
                        None
                    };

                    let right_data = if j < inserts.len() {
                        right_line += 1;
                        Some((right_line - 1, inserts[j].clone()))
                    } else {
                        None
                    };

                    let has_left = left_data.is_some();
                    let has_right = right_data.is_some();

                    // 计算字符级差异（如果左右都有内容）
                    let (left_segments, right_segments) =
                        if let (Some((_, left_content)), Some((_, right_content))) =
                            (&left_data, &right_data)
                        {
                            compute_char_diff(left_content, right_content)
                        } else {
                            (
                                if has_left {
                                    vec![TextSegment {
                                        text: left_data
                                            .as_ref()
                                            .map(|(_, c)| c.clone())
                                            .unwrap_or_default(),
                                        diff_type: DiffType::Removed,
                                    }]
                                } else {
                                    Vec::new()
                                },
                                if has_right {
                                    vec![TextSegment {
                                        text: right_data
                                            .as_ref()
                                            .map(|(_, c)| c.clone())
                                            .unwrap_or_default(),
                                        diff_type: DiffType::Added,
                                    }]
                                } else {
                                    Vec::new()
                                },
                            )
                        };

                    lines.push(SplitLine {
                        left_line_number: left_data.as_ref().map(|(n, _)| *n),
                        left_content: left_data.map(|(_, c)| c),
                        left_type: if has_left {
                            DiffType::Removed
                        } else {
                            DiffType::Equal
                        },
                        left_segments,
                        right_line_number: right_data.as_ref().map(|(n, _)| *n),
                        right_content: right_data.map(|(_, c)| c),
                        right_type: if has_right {
                            DiffType::Added
                        } else {
                            DiffType::Equal
                        },
                        right_segments,
                    });
                }
            }
            ChangeTag::Insert => {
                // 纯插入（没有对应的删除）
                lines.push(SplitLine {
                    left_line_number: None,
                    left_content: None,
                    left_type: DiffType::Equal,
                    left_segments: Vec::new(),
                    right_line_number: Some(right_line),
                    right_content: Some(content.clone()),
                    right_type: DiffType::Added,
                    right_segments: vec![TextSegment {
                        text: content,
                        diff_type: DiffType::Added,
                    }],
                });
                right_line += 1;
                i += 1;
            }
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_text() {
        let left = "hello\nworld";
        let right = "hello\nworld";
        let result = compute_diff(left, right);

        assert_eq!(result.added_count, 0);
        assert_eq!(result.removed_count, 0);
        assert!(result.similarity > 0.99);
    }

    #[test]
    fn test_completely_different() {
        let left = "aaa\nbbb";
        let right = "ccc\nddd";
        let result = compute_diff(left, right);

        assert_eq!(result.added_count, 2);
        assert_eq!(result.removed_count, 2);
    }

    #[test]
    fn test_partial_changes() {
        let left = "line1\nline2\nline3";
        let right = "line1\nmodified\nline3";
        let result = compute_diff(left, right);

        assert_eq!(result.removed_count, 1);
        assert_eq!(result.added_count, 1);
    }

    #[test]
    fn test_empty_text() {
        let left = "";
        let right = "new content";
        let result = compute_diff(left, right);

        assert_eq!(result.added_count, 1);
        assert_eq!(result.removed_count, 0);
        assert_eq!(
            result.diff_hunks,
            vec![DiffHunk {
                start_line: 0,
                end_line: 0,
            }]
        );
    }

    #[test]
    fn test_split_lines_count() {
        let left = "a\nb\nc";
        let right = "a\nd\nc";
        let result = compute_diff(left, right);

        // Split 视图应该有 3 行（a 相同，b->d 修改，c 相同）
        assert_eq!(result.split_lines.len(), 3);
    }

    #[test]
    fn test_diff_hunks_group_consecutive_changes() {
        let left = "same\nold1\nold2\nseparator\nold3\nend";
        let right = "same\nnew1\nnew2\nseparator\nnew3\nend";
        let result = compute_diff(left, right);

        assert_eq!(
            result.diff_hunks,
            vec![
                DiffHunk {
                    start_line: 1,
                    end_line: 2,
                },
                DiffHunk {
                    start_line: 4,
                    end_line: 4,
                },
            ]
        );
        assert_eq!(
            result.unified_diff_hunks,
            vec![
                DiffHunk {
                    start_line: 1,
                    end_line: 4,
                },
                DiffHunk {
                    start_line: 6,
                    end_line: 7,
                },
            ]
        );
    }

    #[test]
    fn test_diff_hunks_empty_for_same_text() {
        let result = compute_diff("same\ntext", "same\ntext");

        assert!(result.diff_hunks.is_empty());
        assert!(result.unified_diff_hunks.is_empty());
    }

    #[test]
    fn test_diff_hunks_for_pure_deletion_at_end() {
        let result = compute_diff("same\nremoved", "same");

        assert_eq!(result.diff_hunks.len(), 1);
        let hunk = &result.diff_hunks[0];
        assert!(hunk.start_line <= hunk.end_line);
        assert_eq!(hunk.end_line, result.split_lines.len() - 1);
    }
}
