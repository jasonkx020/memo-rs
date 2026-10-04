//! 左侧导航：视图与分类。

use memo_core::store::MemoCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    All,
    DueToday,
    Todo,
    Credentials,
    Work,
    Life,
    Finance,
    GenderPrivate,
    Emergency,
    Inspiration,
    Trash,
}

impl NavItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部备忘",
            Self::DueToday => "今天到期",
            Self::Todo => "待办提醒",
            Self::Credentials => "账号证件",
            Self::Work => "工作学习",
            Self::Life => "生活家庭",
            Self::Finance => "财务订阅",
            Self::GenderPrivate => "性别私密",
            Self::Emergency => "应急",
            Self::Inspiration => "灵感",
            Self::Trash => "回收站",
        }
    }

    /// 短图标（emoji / 符号）；字体缺字时界面仍可回退显示标签首字。
    pub fn icon(self) -> &'static str {
        match self {
            Self::All => "📋",
            Self::DueToday => "⏰",
            Self::Todo => "✅",
            Self::Credentials => "🔑",
            Self::Work => "💼",
            Self::Life => "🏠",
            Self::Finance => "💰",
            Self::GenderPrivate => "🔒",
            Self::Emergency => "🚨",
            Self::Inspiration => "💡",
            Self::Trash => "🗑",
        }
    }

    /// 映射到备忘分类；视图项返回 None。
    pub fn category(self) -> Option<MemoCategory> {
        match self {
            Self::All | Self::DueToday | Self::Trash => None,
            Self::Todo => Some(MemoCategory::Todo),
            Self::Credentials => Some(MemoCategory::Credentials),
            Self::Work => Some(MemoCategory::Work),
            Self::Life => Some(MemoCategory::Life),
            Self::Finance => Some(MemoCategory::Finance),
            Self::GenderPrivate => Some(MemoCategory::GenderPrivate),
            Self::Emergency => Some(MemoCategory::Emergency),
            Self::Inspiration => Some(MemoCategory::Inspiration),
        }
    }

    #[allow(dead_code)]
    pub fn is_view_section(self) -> bool {
        matches!(self, Self::All | Self::DueToday)
    }

    #[allow(dead_code)]
    pub fn is_category_section(self) -> bool {
        self.category().is_some()
    }

    pub fn is_trash(self) -> bool {
        matches!(self, Self::Trash)
    }

    pub fn is_gender_private(self) -> bool {
        matches!(self, Self::GenderPrivate)
    }

    /// 新建备忘时默认分类。
    #[allow(dead_code)]
    pub fn new_memo_category(self) -> MemoCategory {
        match self {
            Self::All | Self::DueToday | Self::Trash => MemoCategory::General,
            other => other.category().unwrap_or(MemoCategory::General),
        }
    }

    /// 视图区条目。
    pub const VIEWS: &'static [NavItem] = &[NavItem::All, NavItem::DueToday];

    /// 分类列表（含统一「性别私密」）。
    pub fn categories() -> Vec<NavItem> {
        let mut out: Vec<NavItem> = vec![
            NavItem::Todo,
            NavItem::Credentials,
            NavItem::Work,
            NavItem::Life,
            NavItem::Finance,
            NavItem::Emergency,
            NavItem::Inspiration,
        ];
        let insert_at = out
            .iter()
            .position(|i| *i == NavItem::Emergency)
            .unwrap_or(out.len());
        out.insert(insert_at, NavItem::GenderPrivate);
        out
    }
}
