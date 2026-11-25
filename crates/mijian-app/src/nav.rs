//! 左侧业务导航：今日 / 日历 / 备忘 / 私密 / 节点 / 设置 / 回收站。

use mijian_core::store::MemoCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Today,
    Calendar,
    Credentials,
    GenderPrivate,
    Nodes,
    Settings,
    Trash,
}

impl NavItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "今日",
            Self::Calendar => "日历",
            Self::Credentials => "备忘",
            Self::GenderPrivate => "私密",
            Self::Nodes => "节点",
            Self::Settings => "设置",
            Self::Trash => "回收站",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Today => "⌂",
            Self::Calendar => "📅",
            Self::Credentials => "🔑",
            Self::GenderPrivate => "🔒",
            Self::Nodes => "⬡",
            Self::Settings => "⚙",
            Self::Trash => "🗑",
        }
    }

    pub fn category(self) -> Option<MemoCategory> {
        match self {
            Self::Credentials => Some(MemoCategory::Credentials),
            Self::Today | Self::Calendar => Some(MemoCategory::Todo),
            _ => None,
        }
    }

    pub fn is_trash(self) -> bool {
        matches!(self, Self::Trash)
    }

    pub fn is_gender_private(self) -> bool {
        matches!(self, Self::GenderPrivate)
    }

    pub fn is_calendar(self) -> bool {
        matches!(self, Self::Calendar)
    }

    pub fn is_today(self) -> bool {
        matches!(self, Self::Today)
    }

    pub fn is_credentials(self) -> bool {
        matches!(self, Self::Credentials)
    }

    pub fn is_nodes(self) -> bool {
        matches!(self, Self::Nodes)
    }

    pub fn is_settings(self) -> bool {
        matches!(self, Self::Settings)
    }

    /// 任务类（今日 / 日历）：共用任务抽屉与 Todo 分类。
    pub fn is_task_space(self) -> bool {
        matches!(self, Self::Today | Self::Calendar)
    }

    /// 主区业务页（侧栏上段）。
    pub const PRIMARY: &'static [NavItem] = &[
        NavItem::Today,
        NavItem::Calendar,
        NavItem::Credentials,
        NavItem::GenderPrivate,
    ];

    /// 系统页（侧栏中段）。
    pub const SYSTEM: &'static [NavItem] = &[NavItem::Nodes, NavItem::Settings];

    /// 兼容旧顶栏枚举遍历（含回收站）。
    pub const TABS: &'static [NavItem] = &[
        NavItem::Today,
        NavItem::Calendar,
        NavItem::Credentials,
        NavItem::GenderPrivate,
        NavItem::Trash,
    ];
}
