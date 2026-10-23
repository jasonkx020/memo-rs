//! 后台线程 → UI 消息。

use crate::runtime::AppRuntime;
use crate::update_check::UpdateInfo;
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
    /// 版本检查结果；`silent` 为真时失败不写关于页文案。
    UpdateCheckResult {
        silent: bool,
        result: Result<UpdateInfo, String>,
    },
}
