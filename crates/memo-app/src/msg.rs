//! 后台线程 → UI 消息。

use crate::runtime::AppRuntime;
use memo_core::store::{MemoCategory, MemoPriority};
use std::sync::Arc;

pub enum BgMsg {
    /// 成功时附带会话登录密码
    UnlockResult(Result<(Arc<AppRuntime>, String), String>),
    Error(String),
    Info(String),
    Refresh,
    Deleted,
    CreatedMemo {
        id: String,
        title: String,
        body: String,
        category: MemoCategory,
        due_date: String,
        end_date: String,
        priority: MemoPriority,
        tags: Vec<String>,
    },
}
