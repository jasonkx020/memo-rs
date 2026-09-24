//! 备忘变更时间线与简易 diff。

use eframe::egui::{self, Color32, RichText, ScrollArea};
use memo_core::service::HistoryEvent;

use crate::theme;

pub fn event_type_label(t: &str) -> String {
    match t {
        "CREATE" => "创建".into(),
        "MODIFY" => "修改".into(),
        "DELETE" => "删除".into(),
        _ => t.to_string(),
    }
}

/// 展示时间线列表；返回选中的两条索引（用于下方 diff）。
pub fn show_list(
    ui: &mut egui::Ui,
    events: &[HistoryEvent],
    display_name: &dyn Fn(&str) -> String,
    sel_a: &mut Option<usize>,
    sel_b: &mut Option<usize>,
) {
    if events.is_empty() {
        ui.label(theme::muted_label("本机暂无该备忘的审计记录。").size(13.0));
        return;
    }
    ui.label(
        theme::muted_label("点击选择两条记录进行对比（本机视角 · 节点非用户账号）").small(),
    );
    ui.add_space(6.0);
    ScrollArea::vertical()
        .max_height(220.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, ev) in events.iter().enumerate() {
                let author = display_name(&ev.author_node);
                let sync = if ev.source.is_empty() {
                    "本地".to_string()
                } else {
                    format!("同步自 {}", ev.source)
                };
                let ver = ev
                    .after
                    .as_ref()
                    .map(|s| s.version.to_string())
                    .unwrap_or_else(|| "-".into());
                let title = ev
                    .after
                    .as_ref()
                    .map(|s| s.title.as_str())
                    .unwrap_or("(无)");
                let label = format!(
                    "#{}  {}  v{}  {}  ·  {}  ·  {}",
                    ev.seq,
                    event_type_label(&ev.event_type),
                    ver,
                    short_time(&ev.time),
                    author,
                    sync
                );
                let selected = *sel_a == Some(i) || *sel_b == Some(i);
                let resp = ui.selectable_label(selected, RichText::new(label).size(13.0));
                if resp.clicked() {
                    toggle_sel(sel_a, sel_b, i);
                }
                if resp.hovered() {
                    resp.on_hover_text(format!("标题: {title}\n完整时间: {}", ev.time));
                }
            }
        });
}

fn toggle_sel(a: &mut Option<usize>, b: &mut Option<usize>, i: usize) {
    if *a == Some(i) {
        *a = None;
        return;
    }
    if *b == Some(i) {
        *b = None;
        return;
    }
    if a.is_none() {
        *a = Some(i);
    } else if b.is_none() {
        *b = Some(i);
    } else {
        *a = *b;
        *b = Some(i);
    }
}

fn short_time(rfc: &str) -> String {
    // 2024-01-02T03:04:05.xxxZ → 01-02 03:04:05
    if rfc.len() >= 19 {
        let d = &rfc[5..10];
        let t = &rfc[11..19];
        format!("{d} {t}")
    } else {
        rfc.to_string()
    }
}

pub fn show_diff(ui: &mut egui::Ui, events: &[HistoryEvent], a: Option<usize>, b: Option<usize>) {
    let (Some(ia), Some(ib)) = (a, b) else {
        ui.add_space(8.0);
        ui.label(theme::muted_label("请选择两条记录查看差异。").small());
        return;
    };
    if ia >= events.len() || ib >= events.len() {
        return;
    }
    let (lo, hi) = if ia < ib { (ia, ib) } else { (ib, ia) };
    let older = &events[lo];
    let newer = &events[hi];
    let ob = older.after.as_ref();
    let nb = newer.after.as_ref();

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!(
            "对比 #{} → #{}",
            older.seq, newer.seq
        ))
        .strong()
        .color(theme::TEXT),
    );
    ui.add_space(6.0);

    let ot = ob.map(|s| s.title.as_str()).unwrap_or("");
    let nt = nb.map(|s| s.title.as_str()).unwrap_or("");
    field_diff(ui, "标题", ot, nt);

    let oc = ob.map(|s| s.content.as_str()).unwrap_or("");
    let nc = nb.map(|s| s.content.as_str()).unwrap_or("");
    ui.add_space(8.0);
    ui.label(RichText::new("正文").strong().size(13.0));
    ScrollArea::vertical()
        .max_height(180.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            text_diff_lines(ui, oc, nc);
        });
}

fn field_diff(ui: &mut egui::Ui, name: &str, old: &str, new: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).strong().size(13.0));
        if old == new {
            ui.label(RichText::new("无变化").small().color(theme::TEXT_MUTED));
        } else {
            ui.label(RichText::new("已变更").small().color(theme::WARN));
        }
    });
    if old != new {
        ui.label(RichText::new(format!("− {old}")).color(theme::DANGER).size(13.0));
        ui.label(RichText::new(format!("+ {new}")).color(theme::SUCCESS).size(13.0));
    } else {
        ui.label(RichText::new(old).size(13.0).color(theme::TEXT));
    }
}

fn text_diff_lines(ui: &mut egui::Ui, old: &str, new: &str) {
    if old == new {
        ui.label(theme::muted_label("正文无变化").small());
        if !new.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(new).size(13.0));
        }
        return;
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let max = old_lines.len().max(new_lines.len());
    for i in 0..max {
        let o = old_lines.get(i).copied().unwrap_or("");
        let n = new_lines.get(i).copied().unwrap_or("");
        if o == n {
            if !o.is_empty() {
                ui.label(RichText::new(format!("  {o}")).size(12.5).color(theme::TEXT));
            }
        } else {
            if !o.is_empty() {
                ui.label(
                    RichText::new(format!("− {o}"))
                        .size(12.5)
                        .color(theme::DANGER)
                        .background_color(Color32::from_rgb(255, 236, 236)),
                );
            }
            if !n.is_empty() {
                ui.label(
                    RichText::new(format!("+ {n}"))
                        .size(12.5)
                        .color(theme::SUCCESS)
                        .background_color(Color32::from_rgb(232, 248, 237)),
                );
            }
        }
    }
}
