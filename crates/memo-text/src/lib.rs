//! 可复用的 egui 流式纯文本编辑器（样式 runs + 选区 + 列表 + 嵌入原子）。
//! 不依赖业务 Doc 类型；由宿主负责与文档模型同步。

mod embed;
mod list;
mod style;
mod widget;

pub use embed::{FlowEmbed, FlowEmbedKind, EMBED_CHAR};
pub use list::{
    apply_list_style_to_lines, clamp_cursor_to_list_body, cursor_at_line_body,
    handle_backspace_list, handle_enter, line_at_cursor, line_body_start, line_index_at,
    move_vert_logical, parse_list_prefix, ListStyle,
};
pub use style::{StyleDelta, StyleRun};
pub use widget::{FlowEditor, FlowOutput, FlowSelection, FlowState};
