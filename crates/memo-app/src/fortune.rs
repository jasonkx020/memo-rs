//! 日历运势：命 / 运 / 财 / 衣（本地种子，不联网）。

use chrono::{Datelike, NaiveDate};
use eframe::egui::Color32;
use lunar_rust::lunar::LunarRefHelper;
use lunar_rust::solar::{self, SolarRefHelper};
use memo_core::person::Gender;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FortuneTab {
    Ming,
    #[default]
    Yun,
    Cai,
    Yi,
}

impl FortuneTab {
    pub const ALL: [FortuneTab; 4] = [
        FortuneTab::Ming,
        FortuneTab::Yun,
        FortuneTab::Cai,
        FortuneTab::Yi,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ming => "命",
            Self::Yun => "运",
            Self::Cai => "财",
            Self::Yi => "衣",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FortuneMode {
    Anon,
    Bound,
}

#[derive(Debug, Clone)]
pub struct ClothColor {
    pub color: Color32,
    pub name: &'static str,
    pub vibe: &'static str,
}

#[derive(Debug, Clone)]
pub struct DayFortune {
    pub mode: FortuneMode,
    pub score_overall: u8,
    pub score_wealth: u8,
    pub stars_overall: u8,
    pub stars_wealth: u8,
    pub one_liner: String,
    pub yi: String,
    pub ji: String,
    pub direction: String,
    pub fate_tags: Vec<&'static str>,
    pub fate_blurb: String,
    pub fate_header: String,
    pub wealth_in: String,
    pub wealth_out: String,
    pub cloth: ClothColor,
    pub cloth_top: String,
    pub cloth_bottom: String,
    pub cloth_accent: String,
    pub sheng_xiao: String,
    pub day_gan_zhi: String,
}

struct Rng(u64);

impl Rng {
    fn from_seed(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.0 >> 32) as u32
    }

    fn gen_range(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + self.next_u32() % (hi - lo)
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.gen_range(0, items.len() as u32) as usize]
    }
}

fn seed_hash(fp: &str, day: NaiveDate, birth: &str, hour: Option<u8>) -> u64 {
    let mut h = DefaultHasher::new();
    fp.hash(&mut h);
    day.hash(&mut h);
    if birth.is_empty() {
        "anon".hash(&mut h);
    } else {
        birth.hash(&mut h);
        hour.unwrap_or(255).hash(&mut h);
    }
    h.finish()
}

fn stars_from_score(score: u8) -> u8 {
    match score {
        0..=29 => 1,
        30..=49 => 2,
        50..=69 => 3,
        70..=84 => 4,
        _ => 5,
    }
}

fn stars_label(n: u8) -> String {
    let n = n.clamp(1, 5);
    format!("{}{}", "★".repeat(n as usize), "☆".repeat((5 - n) as usize))
}

pub fn format_stars(n: u8) -> String {
    stars_label(n)
}

const ONE_LINERS: &[&str] = &[
    "午后适合收束决议，傍晚少开新议题。",
    "上午宜对齐预期，下午做一件最难的。",
    "今日宜把清单收束成三件，少开新线。",
    "沟通宜短而准，书面确认胜过口头承诺。",
    "节奏宜稳：先交付再扩展。",
    "适合复盘与清理，不适合强推未成熟方案。",
    "小步推进即可，忌一上来改大方向。",
    "傍晚前收口，夜里留给休息而非新坑。",
];

const YI_BITS: &[&str] = &[
    "沟通确认",
    "整理归档",
    "跟进尾款",
    "轻量复盘",
    "约短会",
    "更新清单",
];

const JI_BITS: &[&str] = &[
    "临时改期",
    "口头承诺交付",
    "冲动下单",
    "同时开多线",
    "深夜决策",
    "情绪化回复",
];

const DIRS: &[&str] = &["偏东", "偏南", "偏西", "偏北", "东南", "西南"];

const FATE_TAGS: &[&str] = &[
    "稳中求进",
    "重承诺",
    "忌急躁",
    "善收口",
    "重证据",
    "宜倾听",
    "守边界",
    "敢拍板",
];

const FATE_BLURBS: &[&str] = &[
    "适合复盘与对齐预期，不宜强推未成熟方案。",
    "处事宜先听后断，口头表态宜留余地。",
    "今日气场偏务实，把一件事做完胜过开三件。",
    "人际上宜清晰边界，少卷入无关争论。",
];

const WEALTH_IN: &[&str] = &[
    "报销/尾款跟进可试",
    "小额进账或退款有望",
    "旧案回款值得轻推一把",
    "合作意向可再确认一次",
];

const WEALTH_OUT: &[&str] = &[
    "忌冲动订阅与分期",
    "大额决策宜再压一夜",
    "少开非必要新开支",
    "避免跟风促销",
];

const CLOTHS: &[ClothColor] = &[
    ClothColor {
        color: Color32::from_rgb(0x3d, 0x6b, 0x8c),
        name: "雾蓝",
        vibe: "冷静可信",
    },
    ClothColor {
        color: Color32::from_rgb(0x5a, 0x6b, 0x4e),
        name: "苔绿",
        vibe: "沉稳务实",
    },
    ClothColor {
        color: Color32::from_rgb(0x8a, 0x6a, 0x4a),
        name: "暖驼",
        vibe: "亲和从容",
    },
    ClothColor {
        color: Color32::from_rgb(0x4a, 0x4f, 0x5c),
        name: "石墨灰",
        vibe: "干净利落",
    },
    ClothColor {
        color: Color32::from_rgb(0x6b, 0x4e, 0x5a),
        name: "豆沙",
        vibe: "柔和得体",
    },
    ClothColor {
        color: Color32::from_rgb(0x3e, 0x5c, 0x5a),
        name: "青灰",
        vibe: "克制有度",
    },
];

fn lunar_birth_labels(birth_ymd: &str) -> (String, String) {
    let Ok(d) = NaiveDate::parse_from_str(birth_ymd.trim(), "%Y-%m-%d") else {
        return (String::new(), String::new());
    };
    let solar = solar::from_ymd(d.year() as i64, d.month() as i64, d.day() as i64);
    let lunar = solar.get_lunar();
    (
        lunar.get_year_sheng_xiao(),
        lunar.get_day_in_gan_zhi(),
    )
}

fn cloth_lines(cloth: &ClothColor, gender: Gender, rng: &mut Rng) -> (String, String, String) {
    let tops = match gender {
        Gender::Female => ["雾感上装", "针织开衫", "利落衬衫"],
        Gender::Male => ["衬衫或针织", "轻薄外套", "干净上装"],
        Gender::Unknown => ["干净上装", "利落外套", "基础款上衣"],
    };
    let bottoms = ["深灰 / 卡其", "深蓝直筒", "中性色长裤"];
    let accents = ["米白或木质色点缀", "金属小配饰点到为止", "浅色袜鞋压低重心"];
    (
        format!("{}或{}", cloth.name, rng.pick(&tops)),
        (*rng.pick(&bottoms)).to_string(),
        (*rng.pick(&accents)).to_string(),
    )
}

/// 按选中日与可选生辰生成当日运势（同输入稳定）。
pub fn for_day(
    day: NaiveDate,
    person_fp: &str,
    birth_ymd: &str,
    birth_hour: Option<u8>,
    gender: Gender,
) -> DayFortune {
    let bound = !birth_ymd.trim().is_empty();
    let mode = if bound {
        FortuneMode::Bound
    } else {
        FortuneMode::Anon
    };
    let mut rng = Rng::from_seed(seed_hash(
        person_fp,
        day,
        if bound { birth_ymd } else { "" },
        if bound { birth_hour } else { None },
    ));

    let score_overall = rng.gen_range(42, 92) as u8;
    let score_wealth = rng.gen_range(38, 88) as u8;
    let stars_overall = stars_from_score(score_overall);
    let stars_wealth = stars_from_score(score_wealth);

    let one_liner = (*rng.pick(ONE_LINERS)).to_string();
    let yi = format!("{} · {}", rng.pick(YI_BITS), rng.pick(YI_BITS));
    let ji = format!("{} · {}", rng.pick(JI_BITS), rng.pick(JI_BITS));
    let direction = (*rng.pick(DIRS)).to_string();

    let mut fate_tags = Vec::new();
    for _ in 0..3 {
        let t = *rng.pick(FATE_TAGS);
        if !fate_tags.contains(&t) {
            fate_tags.push(t);
        }
    }
    while fate_tags.len() < 3 {
        fate_tags.push(*rng.pick(FATE_TAGS));
    }

    let (sheng_xiao, day_gan_zhi) = if bound {
        lunar_birth_labels(birth_ymd)
    } else {
        (String::new(), String::new())
    };

    let fate_header = if bound {
        let mut s = String::new();
        if !day_gan_zhi.is_empty() {
            s.push_str(&format!("日柱{day_gan_zhi}"));
        }
        if !sheng_xiao.is_empty() {
            if !s.is_empty() {
                s.push_str(" · ");
            }
            s.push_str(&format!("生肖{sheng_xiao}"));
        }
        if birth_hour.is_none() {
            if !s.is_empty() {
                s.push_str(" · ");
            }
            s.push_str("时辰未知");
        }
        if s.is_empty() {
            "已绑定生辰".into()
        } else {
            s
        }
    } else {
        "娱乐推演 · 未绑定生辰".into()
    };

    let fate_blurb = if bound && !day_gan_zhi.is_empty() {
        format!(
            "{}，{}",
            day_gan_zhi,
            rng.pick(FATE_BLURBS)
        )
    } else {
        (*rng.pick(FATE_BLURBS)).to_string()
    };

    let wealth_in = (*rng.pick(WEALTH_IN)).to_string();
    let wealth_out = (*rng.pick(WEALTH_OUT)).to_string();
    let cloth = rng.pick(CLOTHS).clone();
    let (cloth_top, cloth_bottom, cloth_accent) = cloth_lines(&cloth, gender, &mut rng);

    DayFortune {
        mode,
        score_overall,
        score_wealth,
        stars_overall,
        stars_wealth,
        one_liner,
        yi,
        ji,
        direction,
        fate_tags,
        fate_blurb,
        fate_header,
        wealth_in,
        wealth_out,
        cloth,
        cloth_top,
        cloth_bottom,
        cloth_accent,
        sheng_xiao,
        day_gan_zhi,
    }
}
