//! 顶栏业务页签：任务 / 备忘 / 私密 / 回收站。

use memo_core::store::MemoCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Calendar,
    Credentials,
    GenderPrivate,
    Trash,
}

impl NavItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::Calendar => "任务",
            Self::Credentials => "备忘",
            Self::GenderPrivate => "私密",
            Self::Trash => "回收站",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Calendar => "📅",
            Self::Credentials => "🔑",
            Self::GenderPrivate => "🔒",
            Self::Trash => "🗑",
        }
    }

    pub fn category(self) -> Option<MemoCategory> {
        match self {
            Self::Calendar | Self::Trash => None,
            Self::Credentials => Some(MemoCategory::Credentials),
            Self::GenderPrivate => Some(MemoCategory::GenderPrivate),
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

    pub const TABS: &'static [NavItem] = &[
        NavItem::Calendar,
        NavItem::Credentials,
        NavItem::GenderPrivate,
        NavItem::Trash,
    ];
}
