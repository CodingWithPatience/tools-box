use serde::{Deserialize, Serialize};

/// 笔记目录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteFolder {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub sort_order: i32,
    pub created_at: String,
}

/// 笔记条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEntry {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub folder_id: Option<i64>,
    pub is_favorite: bool,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 笔记表单（编辑用）
#[derive(Debug, Clone)]
pub struct NoteForm {
    pub title: String,
    pub content: String,
    pub folder_id: Option<i64>,
    pub tags: String,
}

impl NoteForm {
    /// 创建空表单
    pub fn empty() -> Self {
        Self {
            title: String::new(),
            content: String::new(),
            folder_id: None,
            tags: String::new(),
        }
    }

    /// 从笔记条目创建表单
    pub fn from_entry(entry: &NoteEntry) -> Self {
        Self {
            title: entry.title.clone(),
            content: entry.content.clone(),
            folder_id: entry.folder_id,
            tags: entry.tags.join(", "),
        }
    }

    /// 解析标签字符串为 Vec
    pub fn parse_tags(&self) -> Vec<String> {
        self.tags
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// 排序方式
#[derive(Debug, Clone, PartialEq)]
pub enum SortBy {
    CreatedAt,
    UpdatedAt,
    Title,
}

impl SortBy {
    /// 获取所有排序方式
    pub fn all() -> &'static [SortBy] {
        &[SortBy::CreatedAt, SortBy::UpdatedAt, SortBy::Title]
    }

    /// 转换为显示文本
    pub fn as_str(&self) -> &'static str {
        match self {
            SortBy::CreatedAt => "创建时间",
            SortBy::UpdatedAt => "更新时间",
            SortBy::Title => "标题",
        }
    }
}

/// 视图模式
#[derive(Debug, Clone, PartialEq)]
pub enum NoteViewMode {
    Edit,
    Preview,
}