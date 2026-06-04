/// 差异类型
#[derive(Debug, Clone, PartialEq)]
pub enum DiffType {
    /// 相同
    Equal,
    /// 新增
    Added,
    /// 删除
    Removed,
}

/// 文本片段（用于字符级差异显示）
#[derive(Debug, Clone)]
pub struct TextSegment {
    /// 文本内容
    pub text: String,
    /// 差异类型
    pub diff_type: DiffType,
}

/// 视图模式
#[derive(Debug, Clone, PartialEq)]
pub enum ViewMode {
    /// 编辑模式（输入文本）
    Edit,
    /// Split Layout 差异视图（左右并排）
    Split,
    /// Unified 差异视图（传统单栏）
    Unified,
}

/// 单行差异（用于 Unified 视图）
#[derive(Debug, Clone)]
pub struct DiffLine {
    /// 左侧行号（None 表示该行在原始文本中不存在）
    pub line_number_left: Option<usize>,
    /// 右侧行号（None 表示该行在对比文本中不存在）
    pub line_number_right: Option<usize>,
    /// 行内容
    pub content: String,
    /// 差异类型
    pub diff_type: DiffType,
}

/// Split 视图的单行数据
#[derive(Debug, Clone)]
pub struct SplitLine {
    /// 左侧行号（None 表示该行为空）
    pub left_line_number: Option<usize>,
    /// 左侧内容（None 表示该行为空）
    pub left_content: Option<String>,
    /// 左侧差异类型
    pub left_type: DiffType,
    /// 左侧字符级差异片段
    pub left_segments: Vec<TextSegment>,
    /// 右侧行号（None 表示该行为空）
    pub right_line_number: Option<usize>,
    /// 右侧内容（None 表示该行为空）
    pub right_content: Option<String>,
    /// 右侧差异类型
    pub right_type: DiffType,
    /// 右侧字符级差异片段
    pub right_segments: Vec<TextSegment>,
}

/// 差异结果
#[derive(Debug, Clone)]
pub struct DiffResult {
    /// Unified 视图数据
    pub unified_lines: Vec<DiffLine>,
    /// Split 视图数据
    pub split_lines: Vec<SplitLine>,
    /// 新增行数
    pub added_count: usize,
    /// 删除行数
    pub removed_count: usize,
    /// 相似度 (0.0 - 1.0)
    pub similarity: f64,
}

impl DiffResult {
    /// 创建空的差异结果
    pub fn empty() -> Self {
        Self {
            unified_lines: Vec::new(),
            split_lines: Vec::new(),
            added_count: 0,
            removed_count: 0,
            similarity: 1.0,
        }
    }
}
