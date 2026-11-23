//! 桌面应用壳说明。
//!
//! `MemoApp` / `Screen` 与巨大的 `update` 实现仍在 crate 根 [`crate`]（`lib.rs`），
//! 以免与设置/导航/便签等视图形成循环依赖。增量逻辑请优先放到独立模块
//!（如 [`crate::runtime_boot`]、[`crate::settings_view`]、各 `*_view`）。
//!
//! 身份门仅服务**终端用户**；存储节点见 CLI no-ui bootstrap。
