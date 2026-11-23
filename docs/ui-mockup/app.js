(() => {
  "use strict";

  const TITLES = {
    today: "今日",
    calendar: "日历",
    memos: "备忘",
    private: "私密",
    nodes: "节点",
    settings: "设置",
    trash: "回收站",
  };

  const TODAY = "2026-11-23";

  const tasks = [
    {
      id: "t1",
      title: "续写服务器部署说明",
      due: "2026-11-20",
      bucket: "overdue",
      tags: ["文档"],
      priority: "高",
      category: "任务",
      private: false,
      visibility: "节点可见",
      version: 3,
      updated: "昨天 16:40",
      time: null,
      body: "补齐 data_dir 与 cluster_salt 配置说明，避免再踩路径坑。",
      done: false,
    },
    {
      id: "t2",
      title: "核对防火墙 UDP 17000",
      due: "2026-11-21",
      bucket: "overdue",
      tags: ["网络"],
      priority: "普通",
      category: "任务",
      private: false,
      visibility: "节点可见",
      version: 1,
      updated: "11月21日",
      time: "14:00",
      body: "从机发现不到主机时优先查 UDP 与集群盐。",
      done: false,
    },
    {
      id: "t3",
      title: "和主节点联调托管还原",
      due: "2026-11-23",
      bucket: "due",
      tags: ["同步"],
      priority: "高",
      category: "任务",
      private: false,
      visibility: "节点可见",
      version: 2,
      updated: "今天 09:18",
      time: "10:30",
      body: "导入同一 .memokey 后确认 hosted 可还原。",
      done: false,
    },
    {
      id: "t4",
      title: "整理标签体系",
      due: null,
      bucket: "inbox",
      tags: ["整理"],
      priority: "普通",
      category: "任务",
      private: false,
      visibility: "节点可见",
      version: 1,
      updated: "今天 08:02",
      time: null,
      body: "工作 / 生活 / 财务用标签，不再堆分类。",
      done: false,
    },
    {
      id: "t5",
      title: "写一封给自己的周记",
      due: null,
      bucket: "inbox",
      tags: ["生活"],
      priority: "普通",
      category: "任务",
      private: true,
      visibility: "私密 · 本地加密",
      version: 1,
      updated: "昨天 21:10",
      time: null,
      body: "先记着，不定日期。",
      done: false,
    },
    {
      id: "t6",
      title: "月中账单核对",
      due: "2026-11-28",
      bucket: "later",
      tags: ["财务"],
      priority: "普通",
      category: "任务",
      private: false,
      visibility: "节点可见",
      version: 4,
      updated: "11月15日",
      time: null,
      body: "日历上的远期事项。",
      done: false,
    },
  ];

  const memos = [
    {
      id: "m1",
      title: "NAS 管理账号",
      kind: "账号",
      sub: "home.lan · 标签：账号",
      ico: "N",
      updated: "今天 09:12",
      tags: ["网络", "账号"],
      fields: [
        { lab: "地址", val: "https://nas.home.local:5001" },
        { lab: "用户", val: "admin" },
        { lab: "备注", val: "2FA 备用码在保险箱" },
      ],
      body: "用户名 admin。密码在密码管理器。勿局域网共享。",
    },
    {
      id: "m2",
      title: "护照信息",
      kind: "证件",
      sub: "证件 · 到期 2031-04",
      ico: "证",
      updated: "昨天",
      tags: ["证件"],
      fields: [
        { lab: "号码", val: "E12345678" },
        { lab: "有效期", val: "2031-08-12" },
        { lab: "签发地", val: "上海" },
      ],
      body: "护照号已脱敏。扫描件在私密保险箱。",
    },
    {
      id: "m3",
      title: "集群盐备份备忘",
      kind: "网络",
      sub: "标签：网络",
      ico: "盐",
      updated: "11-18",
      tags: ["网络"],
      fields: [
        { lab: "用途", val: "cluster_salt_hex 同集群一致" },
        { lab: "注意", val: "本机 salt_hex 勿外传" },
      ],
      body: "cluster_salt_hex 与各机一致；本机 salt_hex 勿外传。",
    },
  ];

  const privateTasks = [
    {
      id: "pt1",
      title: "预约牙科复查",
      meta: "私密 · 未安排",
      body: "仅本人可见，不参与局域网共享。",
      done: false,
    },
  ];

  const privateMemos = [
    {
      id: "pm1",
      title: "保险柜密码提示",
      kind: "账号",
      sub: "私密备忘",
      ico: "锁",
      updated: "上周",
      tags: ["私密"],
      fields: [{ lab: "提示", val: "纸质本第 12 页" }],
      body: "提示词写在纸质本第 12 页，不写明文。",
    },
  ];

  const trash = [
    {
      id: "tr1",
      title: "旧会议纪要草稿",
      kind: "账号",
      sub: "7 天后彻底清除",
      ico: "废",
      updated: "—",
      tags: [],
      fields: [],
      body: "可恢复或彻底删除。",
    },
  ];

  const peers = [
    {
      id: "node-a1b2c3d4",
      name: "书房主机",
      sub: "主机 · 接受托管 · 在线",
      on: true,
      host: true,
    },
    {
      id: "node-deadbeef",
      name: "客厅备用",
      sub: "从机 · 不稳定",
      on: false,
      host: false,
    },
  ];

  const LUNAR_DAYS = [
    "初一","初二","初三","初四","初五","初六","初七","初八","初九","初十",
    "十一","十二","十三","十四","十五","十六","十七","十八","十九","二十",
    "廿一","廿二","廿三","廿四","廿五","廿六","廿七","廿八","廿九","三十",
  ];
  const JIEQI = { "2026-11-07": "立冬", "2026-11-22": "小雪" };
  const GANZHI = [
    "甲子","乙丑","丙寅","丁卯","戊辰","己巳","庚午","辛未","壬申","癸酉",
    "甲戌","乙亥","丙子","丁丑","戊寅","己卯","庚辰","辛巳","壬午","癸未",
    "甲申","乙酉","丙戌","丁亥","戊子","己丑","庚寅","辛卯","壬辰","癸巳",
    "甲午","乙未","丙申","丁酉","戊戌","己亥","庚子","辛丑","壬寅","癸卯",
    "甲辰","乙巳","丙午","丁未","戊申","己酉","庚戌","辛亥","壬子","癸丑",
    "甲寅","乙卯","丙辰","丁巳","戊午","己未","庚申","辛酉","壬戌","癸亥",
  ];
  const YI_POOL = ["推进重点", "整理文档", "短会确认", "跟进尾款", "更新清单", "轻量复盘", "沟通确认", "归档资料"];
  const JI_POOL = ["临时加会", "口头承诺", "深夜决策", "同时开多线", "冲动下单", "情绪化回复", "临时改期", "强推未成熟方案"];
  const ONE_LINERS = [
    "午后适合收束决议，傍晚少开新议题。",
    "上午宜对齐预期，下午做一件最难的。",
    "今日宜把清单收束成三件，少开新线。",
    "沟通宜短而准，书面确认胜过口头承诺。",
    "节奏宜稳：先交付再扩展。",
    "适合复盘与清理，不适合强推未成熟方案。",
  ];
  const CLOTHS = [
    { hex: "#3d6b8c", name: "雾蓝", vibe: "冷静可信" },
    { hex: "#5a6b4e", name: "苔绿", vibe: "沉稳务实" },
    { hex: "#8a6a4a", name: "暖驼", vibe: "亲和从容" },
    { hex: "#4a4f5c", name: "石墨灰", vibe: "干净利落" },
    { hex: "#6b4e5a", name: "豆沙", vibe: "柔和得体" },
  ];

  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

  const pageTitle = $("#page-title");
  const searchInput = $("#search");
  const overlay = $("#overlay");
  const drawerDetail = $("#drawer-detail");
  const drawerCreate = $("#drawer-create");
  const lockTimer = $("#lock-timer");

  let currentView = "today";
  let selectedDay = new Date(2026, 10, 23);
  let calCursor = new Date(2026, 10, 1);
  let createKind = "task";
  let createWhen = "today";
  let lastDetail = null;
  let fortuneTab = "yun";
  let memoFilter = "all";
  let toastEl = null;

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function showToast(msg) {
    if (!toastEl) {
      toastEl = document.createElement("div");
      toastEl.className = "toast";
      $(".main").appendChild(toastEl);
    }
    toastEl.textContent = msg;
    toastEl.classList.add("show");
    clearTimeout(showToast._t);
    showToast._t = setTimeout(() => toastEl.classList.remove("show"), 1600);
  }

  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(
        () => showToast("已复制"),
        () => showToast("复制失败")
      );
    } else {
      showToast("已复制（示意）");
    }
  }

  function fmtDateCN(d) {
    const weeks = ["日", "一", "二", "三", "四", "五", "六"];
    return `${d.getMonth() + 1} 月 ${d.getDate()} 日 · 周${weeks[d.getDay()]}`;
  }

  function iso(d) {
    const m = `${d.getMonth() + 1}`.padStart(2, "0");
    const day = `${d.getDate()}`.padStart(2, "0");
    return `${d.getFullYear()}-${m}-${day}`;
  }

  function daySeed(dateStr) {
    let h = 2166136261;
    for (let i = 0; i < dateStr.length; i++) {
      h ^= dateStr.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return h >>> 0;
  }

  function seededPick(seed, arr, offset) {
    return arr[(seed + (offset || 0)) % arr.length];
  }

  function starsLabel(n) {
    const s = Math.max(1, Math.min(5, n));
    return "★".repeat(s) + "☆".repeat(5 - s);
  }

  function lunarCellMark(dateStr) {
    if (JIEQI[dateStr]) return { text: JIEQI[dateStr], jieqi: true };
    const d = Number(dateStr.slice(-2));
    return { text: LUNAR_DAYS[(d - 1) % LUNAR_DAYS.length], jieqi: false };
  }

  function huangliFor(dateStr) {
    const seed = daySeed(dateStr);
    const d = Number(dateStr.slice(-2));
    const lunarDay = LUNAR_DAYS[(d - 1) % LUNAR_DAYS.length];
    const gz = GANZHI[seed % GANZHI.length];
    const jq = JIEQI[dateStr];
    let lunar = `农历十月${lunarDay} · ${gz}`;
    if (jq) lunar += ` · ${jq}`;
    return {
      lunar,
      yi: [0, 1, 2].map((i) => seededPick(seed, YI_POOL, i * 3)).join("、"),
      ji: [0, 1, 2].map((i) => seededPick(seed, JI_POOL, i * 5)).join("、"),
    };
  }

  function fortuneFor(dateStr) {
    const seed = daySeed(dateStr);
    const score = 42 + (seed % 48);
    const wealth = 38 + ((seed >> 3) % 50);
    const stars = score >= 85 ? 5 : score >= 70 ? 4 : score >= 50 ? 3 : score >= 30 ? 2 : 1;
    const wStars = wealth >= 85 ? 5 : wealth >= 70 ? 4 : wealth >= 50 ? 3 : wealth >= 30 ? 2 : 1;
    const cloth = CLOTHS[seed % CLOTHS.length];
    return {
      score,
      wealth,
      stars,
      wStars,
      oneLiner: seededPick(seed, ONE_LINERS, 0),
      yi: `${seededPick(seed, YI_POOL, 1)}、${seededPick(seed, YI_POOL, 4)}`,
      ji: `${seededPick(seed, JI_POOL, 2)}、${seededPick(seed, JI_POOL, 5)}`,
      direction: ["偏东", "偏南", "东南", "偏西", "西南", "偏北"][seed % 6],
      fateHeader: "娱乐模式 · 未绑定生日",
      fateTags: ["稳中求进", "善收口", "忌急躁"],
      fateBlurb: "今日气场偏务实，把一件事做完胜过开三件。",
      wealthIn: "报销/尾款跟进可试",
      wealthOut: "忌冲动订阅与分期",
      cloth,
      clothTop: `${cloth.name}或干净上装`,
      clothBottom: "深灰 / 卡其",
      clothAccent: "米白或木质色点缀",
    };
  }

  function fortuneCardHtml(f, tab) {
    let inner = "";
    if (tab === "ming") {
      inner = `
        <div class="fortune-sub">${escapeHtml(f.fateHeader)}</div>
        <div class="fortune-tags">
          ${f.fateTags.map((t, i) => `<span class="t${i % 3}">${escapeHtml(t)}</span>`).join("")}
        </div>
        <div class="fortune-line">${escapeHtml(f.fateBlurb)}</div>`;
    } else if (tab === "yun") {
      inner = `
        <div class="fortune-score-row">
          <span class="lab">综合</span>
          <span class="stars">${starsLabel(f.stars)}</span>
          <span class="score">${f.score}</span>
        </div>
        <div class="fortune-line">${escapeHtml(f.oneLiner)}</div>
        <div class="fortune-meta"><span class="k">宜</span><span>${escapeHtml(f.yi)}</span></div>
        <div class="fortune-meta"><span class="k">忌</span><span>${escapeHtml(f.ji)}</span></div>
        <div class="fortune-meta"><span class="k">方位</span><span>${escapeHtml(f.direction)}</span></div>`;
    } else if (tab === "cai") {
      inner = `
        <div class="fortune-score-row">
          <span class="lab">财运</span>
          <span class="stars">${starsLabel(f.wStars)}</span>
          <span class="score">${f.wealth}</span>
        </div>
        <div class="fortune-line">${escapeHtml(f.oneLiner)}</div>
        <div class="fortune-meta"><span class="k">进</span><span>${escapeHtml(f.wealthIn)}</span></div>
        <div class="fortune-meta"><span class="k">守</span><span>${escapeHtml(f.wealthOut)}</span></div>`;
    } else {
      inner = `
        <div class="cloth-row">
          <div class="cloth-swatch" style="background:${f.cloth.hex}"></div>
          <div>
            <div class="cloth-name">${escapeHtml(f.cloth.name)}</div>
            <div class="cloth-vibe">主色 · ${escapeHtml(f.cloth.vibe)}</div>
          </div>
        </div>
        <div class="fortune-meta"><span class="k">上</span><span>${escapeHtml(f.clothTop)}</span></div>
        <div class="fortune-meta"><span class="k">下</span><span>${escapeHtml(f.clothBottom)}</span></div>
        <div class="fortune-meta"><span class="k">点</span><span>${escapeHtml(f.clothAccent)}</span></div>`;
    }
    return `${inner}
      <div class="fortune-hint">填写生日后推演更准。<a data-go-settings>去设置</a></div>
      <div class="fortune-note">仅供娱乐，非专业预测。</div>`;
  }

  function renderHuangliFortune() {
    const key = iso(selectedDay);
    const hl = huangliFor(key);
    const lunarEl = $("#huangli-lunar");
    const yiEl = $("#huangli-yi");
    const jiEl = $("#huangli-ji");
    if (lunarEl) lunarEl.textContent = hl.lunar;
    if (yiEl) yiEl.textContent = hl.yi;
    if (jiEl) jiEl.textContent = hl.ji;

    $$("#fortune-tabs .ftab").forEach((b) =>
      b.classList.toggle("on", b.dataset.ftab === fortuneTab)
    );
    const f = fortuneFor(key);
    const body = $("#fortune-body");
    if (body) body.innerHTML = fortuneCardHtml(f, fortuneTab);

    // 今日页折叠区同源
    const todayRef = $("#today-ref");
    if (todayRef) {
      const tf = fortuneFor(TODAY);
      const th = huangliFor(TODAY);
      todayRef.innerHTML = `
        <div class="huangli">
          <div class="huangli-lunar">${escapeHtml(th.lunar)}</div>
          <div class="huangli-row"><span class="yi-lab">宜</span><span class="yi-txt">${escapeHtml(th.yi)}</span></div>
          <div class="huangli-row"><span class="ji-lab">忌</span><span class="ji-txt">${escapeHtml(th.ji)}</span></div>
        </div>
        <div class="fortune">
          <div class="fortune-head">
            <div class="fortune-title">今日运势</div>
          </div>
          <div class="fortune-card">${fortuneCardHtml(tf, "yun")}</div>
        </div>`;
    }
  }

  function checkSvg() {
    return `<svg viewBox="0 0 24 24"><path d="M5 12.5 10 17l9-10"/></svg>`;
  }

  function taskButton(t, opts = {}) {
    const pills = [];
    if (t.bucket === "overdue") pills.push(`<span class="pill pill-danger">过期</span>`);
    if (t.bucket === "due") pills.push(`<span class="pill pill-warn">今日</span>`);
    if (t.priority && t.priority !== "普通")
      pills.push(`<span class="pill pill-accent">${t.priority}</span>`);
    if (t.due) pills.push(`<span>${t.due}</span>`);
    (t.tags || []).forEach((tag) => pills.push(`<span class="pill pill-mute">#${tag}</span>`));

    const detailPayload = {
      kind: "task",
      id: t.id,
      title: t.title,
      body: t.body,
      due: t.due || "未安排",
      priority: t.priority || "普通",
      tags: (t.tags || []).join("、") || "—",
      category: t.category || "任务",
      private: !!t.private,
      visibility: t.visibility || "节点可见",
      version: t.version || 1,
      updated: t.updated || "—",
      time: t.time || null,
      done: !!t.done,
    };

    return `
      <button class="task${t.done ? " done" : ""}" type="button" data-task-id="${t.id}" ${
      opts.detail !== false
        ? `data-open-detail='${JSON.stringify(detailPayload).replace(/'/g, "&#39;")}'`
        : ""
    }>
        <span class="check" data-toggle-done="${t.id}" title="完成">${checkSvg()}</span>
        <span class="task-body">
          <span class="task-title">${t.title}</span>
          <span class="task-meta">${pills.join("")}</span>
        </span>
      </button>`;
  }

  function memoCard(m) {
    return `
      <button class="memo-card" type="button" data-open-detail='${JSON.stringify({
        kind: "memo",
        id: m.id,
        title: m.title,
        body: m.body,
        due: "不上日历",
        priority: "—",
        tags: (m.tags || []).join("、") || m.sub || "—",
        memoKind: m.kind || "备忘",
        ico: m.ico,
        updated: m.updated || "—",
        fields: m.fields || [],
      }).replace(/'/g, "&#39;")}'>
        <span class="memo-ico">${m.ico}</span>
        <span>
          <div class="memo-title">${m.title}</div>
          <div class="memo-sub">${m.sub}</div>
        </span>
        <span class="pill pill-mute">${m.kind || "打开"}</span>
      </button>`;
  }

  function emptyHtml(text = "这里暂时没有内容") {
    return `<div class="empty">${text}</div>`;
  }

  function renderToday() {
    const q = searchInput.value.trim().toLowerCase();
    const match = (t) =>
      !q ||
      t.title.toLowerCase().includes(q) ||
      (t.tags || []).some((x) => x.toLowerCase().includes(q));

    const overdue = tasks.filter((t) => t.bucket === "overdue" && match(t));
    const due = tasks.filter((t) => t.bucket === "due" && match(t));
    const inbox = tasks.filter((t) => t.bucket === "inbox" && match(t));

    $("#list-overdue").innerHTML = overdue.map((t) => taskButton(t)).join("") || emptyHtml();
    $("#list-due").innerHTML = due.map((t) => taskButton(t)).join("") || emptyHtml();
    $("#list-inbox").innerHTML = inbox.map((t) => taskButton(t)).join("") || emptyHtml();

    $("[data-count-overdue]").textContent = String(overdue.length);
    $("[data-count-due]").textContent = String(due.length);
    $("[data-count-inbox]").textContent = String(inbox.length);

    const openCount = tasks.filter(
      (t) => !t.done && (t.bucket === "overdue" || t.bucket === "due")
    ).length;
    $("#badge-today").textContent = String(openCount);
    $("#badge-today").style.display = openCount ? "" : "none";
    renderHuangliFortune();
  }

  function tasksOnDay(d) {
    const key = iso(d);
    return tasks.filter((t) => t.due === key);
  }

  function dueSummary(data) {
    if (!data.due || data.due === "未安排") return "未设置日期";
    if (data.time) return `${data.due} · ${data.time}`;
    return `${data.due} · 全天`;
  }

  function statusBadge(data) {
    if (data.done) return { cls: "pill-ok", text: "已完成" };
    if (data.due && data.due < TODAY) return { cls: "pill-danger", text: "已过期" };
    if (data.due === TODAY) return { cls: "pill-warn", text: "今天到期" };
    return { cls: "pill-accent", text: data.kind === "memo" ? "永久保留 · 不上日历" : "日历任务" };
  }

  function renderCalendar() {
    const y = calCursor.getFullYear();
    const m = calCursor.getMonth();
    $("#cal-month").textContent = `${y} 年 ${m + 1} 月`;

    const first = new Date(y, m, 1);
    const startPad = (first.getDay() + 6) % 7;
    const daysInMonth = new Date(y, m + 1, 0).getDate();
    const prevDays = new Date(y, m, 0).getDate();

    const cells = [];
    for (let i = 0; i < 7; i++) {
      cells.push(`<div class="cal-dow">${["一", "二", "三", "四", "五", "六", "日"][i]}</div>`);
    }

    for (let i = 0; i < 42; i++) {
      let dayNum;
      let date;
      let muted = false;
      if (i < startPad) {
        dayNum = prevDays - startPad + i + 1;
        date = new Date(y, m - 1, dayNum);
        muted = true;
      } else if (i >= startPad + daysInMonth) {
        dayNum = i - startPad - daysInMonth + 1;
        date = new Date(y, m + 1, dayNum);
        muted = true;
      } else {
        dayNum = i - startPad + 1;
        date = new Date(y, m, dayNum);
      }

      const key = iso(date);
      const marks = tasks.filter((t) => t.due === key).length;
      const isSel = iso(selectedDay) === key;
      const isToday = key === TODAY;
      const mark = lunarCellMark(key);
      cells.push(`
        <button type="button" class="cal-day${muted ? " muted" : ""}${isSel ? " selected" : ""}${
        isToday ? " today" : ""
      }" data-date="${key}">
          <span class="num">${dayNum}</span>
          <span class="lunar${mark.jieqi ? " jieqi" : ""}">${mark.text}</span>
          <span class="cal-marks">${marks ? "<i></i>".repeat(Math.min(marks, 3)) : ""}</span>
        </button>`);
    }
    $("#cal-grid").innerHTML = cells.join("");

    $("#day-title").textContent = `${selectedDay.getMonth() + 1} 月 ${selectedDay.getDate()} 日`;
    const dayTasks = tasksOnDay(selectedDay);
    const weeks = ["日", "一", "二", "三", "四", "五", "六"];
    $("#day-sub").textContent = dayTasks.length
      ? `周${weeks[selectedDay.getDay()]} · ${dayTasks.length} 项`
      : `周${weeks[selectedDay.getDay()]} · 这天没有任务`;
    $("#list-day").innerHTML =
      dayTasks.map((t) => taskButton(t)).join("") || emptyHtml("这天没有安排");

    renderHuangliFortune();
  }

  function renderMemos() {
    const q = searchInput.value.trim().toLowerCase();
    const list = memos.filter((m) => {
      if (memoFilter !== "all" && m.kind !== memoFilter) return false;
      return (
        !q ||
        m.title.toLowerCase().includes(q) ||
        (m.sub || "").toLowerCase().includes(q) ||
        (m.tags || []).some((t) => t.toLowerCase().includes(q))
      );
    });
    $("#list-memos").innerHTML = list.map(memoCard).join("") || emptyHtml();
  }

  function renderPrivate() {
    $("#list-private-tasks").innerHTML =
      privateTasks
        .map(
          (t) => `
      <button class="task${t.done ? " done" : ""}" type="button" data-open-detail='${JSON.stringify({
        kind: "task",
        title: t.title,
        body: t.body,
        due: "私密",
        priority: "普通",
        tags: "私密空间",
        category: "任务",
        private: true,
        visibility: "私密 · 本地加密",
        version: 1,
        updated: "—",
        done: !!t.done,
      }).replace(/'/g, "&#39;")}'>
        <span class="check" data-toggle-private="${t.id}">${checkSvg()}</span>
        <span class="task-body">
          <span class="task-title">${t.title}</span>
          <span class="task-meta"><span class="pill pill-mute">${t.meta}</span></span>
        </span>
      </button>`
        )
        .join("") || emptyHtml();

    $("#list-private-memos").innerHTML = privateMemos.map(memoCard).join("") || emptyHtml();
  }

  function renderPeers() {
    $("#peer-list").innerHTML = peers
      .map(
        (p) => `
      <div class="peer">
        <span class="st ${p.on ? "on" : "off"}"></span>
        <div>
          <div class="name">${p.name}</div>
          <div class="sub">${p.sub} · ${p.id}</div>
        </div>
        <label class="toggle">
          <input type="checkbox" ${p.host ? "checked" : ""} ${p.host ? "" : "disabled"} />
          托管
        </label>
      </div>`
      )
      .join("");
  }

  function renderTrash() {
    $("#list-trash").innerHTML = trash.map(memoCard).join("") || emptyHtml("回收站是空的");
  }

  function renderAll() {
    renderToday();
    renderCalendar();
    renderMemos();
    renderPrivate();
    renderPeers();
    renderTrash();
  }

  function setView(name) {
    currentView = name;
    $$(".nav-btn[data-view]").forEach((btn) => {
      btn.classList.toggle("active", btn.dataset.view === name);
    });
    $$(".view").forEach((v) => {
      v.classList.toggle("active", v.id === `view-${name}`);
    });
    pageTitle.textContent = TITLES[name] || name;
    closeDrawers();
  }

  function openDrawer(el) {
    overlay.hidden = false;
    requestAnimationFrame(() => {
      overlay.classList.add("open");
      el.classList.add("open");
      el.setAttribute("aria-hidden", "false");
    });
  }

  function closeDrawers() {
    overlay.classList.remove("open");
    [drawerDetail, drawerCreate].forEach((d) => {
      d.classList.remove("open");
      d.setAttribute("aria-hidden", "true");
    });
    setTimeout(() => {
      if (!drawerDetail.classList.contains("open") && !drawerCreate.classList.contains("open")) {
        overlay.hidden = true;
      }
    }, 280);
  }

  function showDetail(data) {
    lastDetail = data;
    const body = $("#detail-rich-body");
    const kicker = $("#detail-kicker");
    const copyBtn = $("#btn-copy-from-detail");
    drawerCreate.classList.remove("open");

    if (data.kind === "memo") {
      kicker.textContent = "备忘详情";
      copyBtn.hidden = false;
      const fields = (data.fields || [])
        .map(
          (f) => `
        <div class="copy-row">
          <span class="lab">${escapeHtml(f.lab)}</span>
          <span class="val">${escapeHtml(f.val)}</span>
          <button type="button" class="mini" data-copy="${escapeHtml(f.val)}">复制</button>
        </div>`
        )
        .join("");
      body.innerHTML = `
        <div class="detail-toolbar">
          <span class="pill pill-ok">永久保留 · 不上日历</span>
          <div class="ops">
            <button type="button" class="btn-sm primary" data-act="edit">编辑</button>
            <button type="button" class="btn-sm" data-act="copy-all">复制</button>
            <button type="button" class="btn-sm" data-act="sticky">便签</button>
            <button type="button" class="btn-sm danger" data-act="delete">删除</button>
          </div>
        </div>
        <div class="memo-hero">
          <div class="big-ico">${escapeHtml(data.ico || "备")}</div>
          <div>
            <h2>${escapeHtml(data.title)}</h2>
            <div class="sub">${escapeHtml(data.memoKind || "备忘")} · 更新 ${escapeHtml(data.updated || "—")}</div>
            <div class="detail-pills">
              <span class="pill pill-accent">${escapeHtml(data.memoKind || "备忘")}</span>
              ${(data.tags || "")
                .split("、")
                .filter(Boolean)
                .map((t) => `<span class="pill pill-mute">#${escapeHtml(t)}</span>`)
                .join("")}
            </div>
          </div>
        </div>
        <div class="detail-card">
          <h3>可复制字段</h3>
          ${fields || `<p class="detail-body-text">暂无结构化字段</p>`}
        </div>
        <div class="detail-card">
          <h3>正文</h3>
          <div class="detail-body-text">${escapeHtml(data.body || "")}</div>
        </div>`;
    } else if (data.kind === "care") {
      kicker.textContent = "关怀详情";
      copyBtn.hidden = true;
      body.innerHTML = `
        <div class="detail-toolbar">
          <span class="pill pill-accent">关怀</span>
        </div>
        <div class="detail-hero"><h2>${escapeHtml(data.title)}</h2></div>
        <hr class="detail-rule" />
        <div class="detail-body-text">${escapeHtml(data.body || "")}</div>`;
    } else {
      kicker.textContent = "任务详情";
      copyBtn.hidden = false;
      const badge = statusBadge(data);
      body.innerHTML = `
        <div class="detail-toolbar">
          <span class="pill ${badge.cls}">${badge.text}</span>
          <div class="ops">
            <button type="button" class="btn-sm primary" data-act="edit">编辑</button>
            <button type="button" class="btn-sm" data-act="copy-all">复制</button>
            <button type="button" class="btn-sm" data-act="sticky">便签</button>
            <button type="button" class="btn-sm danger" data-act="delete">删除</button>
          </div>
        </div>
        <div class="detail-meta-line">${escapeHtml(data.category || "任务")} · 更新于 ${escapeHtml(data.updated || "—")}</div>
        <div class="detail-hero">
          <h2 class="${data.done ? "done" : ""}">${escapeHtml(data.title || "(无标题)")}</h2>
          <div class="detail-pills">
            <span class="pill pill-accent">${escapeHtml(data.category || "任务")}</span>
            <span class="pill ${data.priority === "高" || data.priority === "紧急" ? "pill-danger" : "pill-mute"}">${escapeHtml(data.priority || "普通")}</span>
            ${data.private ? `<span class="pill pill-accent">私</span>` : ""}
            ${(data.tags || "")
              .split("、")
              .filter((t) => t && t !== "—")
              .map((t) => `<span class="pill pill-mute">#${escapeHtml(t)}</span>`)
              .join("")}
          </div>
        </div>
        <hr class="detail-rule" />
        <div class="detail-body-text">${escapeHtml(data.body || "（尚无正文）")}</div>
        <hr class="detail-rule" />
        <div class="meta-cells">
          <div class="meta-cell"><div class="k">到期</div><div class="v">${escapeHtml(dueSummary(data))}</div></div>
          <div class="meta-cell"><div class="k">可见性</div><div class="v">${escapeHtml(data.visibility || "节点可见")}</div></div>
          <div class="meta-cell"><div class="k">版本</div><div class="v">v${data.version || 1} · ${escapeHtml(data.priority || "普通")}</div></div>
        </div>
        <div class="detail-actions">
          <button type="button" class="btn btn-ghost" data-act="reschedule">改期</button>
          <button type="button" class="btn ${data.done ? "btn-ghost" : "btn-primary"}" data-act="toggle-done">
            ${data.done ? "撤销完成" : "标记完成"}
          </button>
        </div>
        <div class="reschedule-panel" id="reschedule-panel" hidden>
          <div class="lab">选择新的到期时间</div>
          <input type="date" id="reschedule-input" value="${data.due && data.due !== "未安排" && data.due !== "私密" ? data.due : TODAY}" />
          <div class="row">
            <button type="button" class="btn btn-ghost" data-act="reschedule-cancel">取消</button>
            <button type="button" class="btn btn-primary" data-act="reschedule-ok">确定改期</button>
          </div>
        </div>`;
    }

    openDrawer(drawerDetail);
  }

  function openCreate() {
    drawerDetail.classList.remove("open");
    $("#create-title").value = "";
    $("#create-body").value = "";
    createKind = "task";
    createWhen = "today";
    $$("[data-kind]").forEach((b) => b.classList.toggle("on", b.dataset.kind === "task"));
    $$("[data-when]").forEach((b) => b.classList.toggle("on", b.dataset.when === "today"));
    updateStepBars();
    openDrawer(drawerCreate);
  }

  function updateStepBars() {
    const hasTitle = $("#create-title").value.trim().length > 0;
    $$("[data-step-bar]").forEach((el) => {
      const n = Number(el.dataset.stepBar);
      el.classList.toggle("on", n === 1 || n === 2 || (n === 3 && hasTitle));
    });
  }

  // Nav
  $$(".nav-btn[data-view]").forEach((btn) => {
    btn.addEventListener("click", () => setView(btn.dataset.view));
  });

  $("#btn-new").addEventListener("click", openCreate);
  $$("[data-open-create]").forEach((b) => b.addEventListener("click", openCreate));
  $("#day-add").addEventListener("click", openCreate);

  $("#btn-lock").addEventListener("click", () => {
    alert("效果稿：将回到身份解锁页（本机登录密码）。");
  });

  searchInput.addEventListener("input", () => {
    if (currentView === "today") renderToday();
    if (currentView === "memos") renderMemos();
    if (currentView === "calendar") renderCalendar();
  });

  $("#content").addEventListener("click", (e) => {
    if (e.target.closest("[data-go-settings]")) {
      e.preventDefault();
      setView("settings");
      $$("[data-settings]").forEach((b) => b.classList.toggle("active", b.dataset.settings === "local"));
      $$(".settings-pane").forEach((p) => p.classList.toggle("active", p.id === "set-local"));
      return;
    }

    const doneBtn = e.target.closest("[data-toggle-done]");
    if (doneBtn) {
      e.stopPropagation();
      const id = doneBtn.dataset.toggleDone;
      const t = tasks.find((x) => x.id === id);
      if (t) {
        t.done = !t.done;
        renderToday();
        renderCalendar();
      }
      return;
    }

    const privDone = e.target.closest("[data-toggle-private]");
    if (privDone) {
      e.stopPropagation();
      const id = privDone.dataset.togglePrivate;
      const t = privateTasks.find((x) => x.id === id);
      if (t) {
        t.done = !t.done;
        renderPrivate();
      }
      return;
    }

    const dayBtn = e.target.closest("[data-date]");
    if (dayBtn) {
      const [yy, mm, dd] = dayBtn.dataset.date.split("-").map(Number);
      selectedDay = new Date(yy, mm - 1, dd);
      renderCalendar();
      return;
    }

    const detailBtn = e.target.closest("[data-open-detail]");
    if (detailBtn) {
      try {
        showDetail(JSON.parse(detailBtn.getAttribute("data-open-detail")));
      } catch (_) {}
      return;
    }

    const care = e.target.closest("[data-detail]");
    if (care) {
      try {
        showDetail(JSON.parse(care.getAttribute("data-detail")));
      } catch (_) {}
    }
  });

  $("#cal-prev").addEventListener("click", () => {
    calCursor = new Date(calCursor.getFullYear(), calCursor.getMonth() - 1, 1);
    renderCalendar();
  });
  $("#cal-next").addEventListener("click", () => {
    calCursor = new Date(calCursor.getFullYear(), calCursor.getMonth() + 1, 1);
    renderCalendar();
  });
  $("#cal-today").addEventListener("click", () => {
    selectedDay = new Date(2026, 10, 23);
    calCursor = new Date(2026, 10, 1);
    renderCalendar();
  });

  $("#fortune-tabs").addEventListener("click", (e) => {
    const tab = e.target.closest(".ftab");
    if (!tab) return;
    fortuneTab = tab.dataset.ftab;
    renderHuangliFortune();
  });

  $("#memo-chips").addEventListener("click", (e) => {
    const chip = e.target.closest(".chip");
    if (!chip) return;
    memoFilter = chip.dataset.filter;
    $$("#memo-chips .chip").forEach((c) => c.classList.toggle("on", c === chip));
    renderMemos();
  });

  $$("[data-private-tab]").forEach((btn) => {
    btn.addEventListener("click", () => {
      $$("[data-private-tab]").forEach((b) => b.classList.toggle("active", b === btn));
      const tab = btn.dataset.privateTab;
      $("#private-care").hidden = tab !== "care";
      $("#private-tasks").hidden = tab !== "tasks";
      $("#private-memos").hidden = tab !== "memos";
    });
  });

  $$("[data-settings]").forEach((btn) => {
    btn.addEventListener("click", () => {
      $$("[data-settings]").forEach((b) => b.classList.toggle("active", b === btn));
      $$(".settings-pane").forEach((p) =>
        p.classList.toggle("active", p.id === `set-${btn.dataset.settings}`)
      );
    });
  });

  $("#btn-sync").addEventListener("click", () => {
    const toast = $("#sync-toast");
    toast.classList.add("show");
    toast.textContent = "已刷新节点列表 · 私人备份已推送";
    setTimeout(() => toast.classList.remove("show"), 2200);
  });

  $("#btn-empty-trash").addEventListener("click", () => {
    if (confirm("效果稿：清空回收站？")) {
      trash.length = 0;
      renderTrash();
    }
  });

  overlay.addEventListener("click", closeDrawers);
  $$("[data-close-drawer]").forEach((b) => b.addEventListener("click", closeDrawers));
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeDrawers();
  });

  $("#detail-rich-body").addEventListener("click", (e) => {
    const copy = e.target.closest("[data-copy]");
    if (copy) {
      copyText(copy.dataset.copy);
      return;
    }
    const actBtn = e.target.closest("[data-act]");
    if (!actBtn || !lastDetail) return;
    const act = actBtn.dataset.act;
    if (act === "edit") {
      $("#btn-edit-from-detail").click();
      return;
    }
    if (act === "copy-all") {
      const lines = [lastDetail.title];
      (lastDetail.fields || []).forEach((f) => lines.push(`${f.lab}：${f.val}`));
      lines.push("", lastDetail.body || "");
      copyText(lines.join("\n"));
      return;
    }
    if (act === "sticky") {
      showToast("已生成桌面便签（示意）");
      return;
    }
    if (act === "delete") {
      showToast("移入回收站（示意）");
      return;
    }
    if (act === "toggle-done" && lastDetail.id) {
      const t = tasks.find((x) => x.id === lastDetail.id);
      if (t) {
        t.done = !t.done;
        lastDetail.done = t.done;
        renderToday();
        renderCalendar();
        if (t.done) {
          closeDrawers();
          showToast("已标为完成");
        } else {
          showDetail({ ...lastDetail, done: false });
          showToast("已撤销完成");
        }
      }
      return;
    }
    if (act === "reschedule") {
      const panel = $("#reschedule-panel");
      if (panel) panel.hidden = !panel.hidden;
      return;
    }
    if (act === "reschedule-cancel") {
      const panel = $("#reschedule-panel");
      if (panel) panel.hidden = true;
      return;
    }
    if (act === "reschedule-ok") {
      const input = $("#reschedule-input");
      const t = lastDetail.id ? tasks.find((x) => x.id === lastDetail.id) : null;
      if (t && input && input.value) {
        t.due = input.value;
        t.bucket = input.value < TODAY ? "overdue" : input.value === TODAY ? "due" : "later";
        t.updated = "今天 刚刚";
        t.version = (t.version || 1) + 1;
        lastDetail.due = t.due;
        lastDetail.updated = t.updated;
        lastDetail.version = t.version;
        renderToday();
        renderCalendar();
        showDetail(lastDetail);
        showToast("已改期");
      }
    }
  });

  $$("[data-kind]").forEach((btn) => {
    btn.addEventListener("click", () => {
      createKind = btn.dataset.kind;
      $$("[data-kind]").forEach((b) => b.classList.toggle("on", b === btn));
      if (createKind === "memo") {
        createWhen = "none";
        $$("[data-when]").forEach((b) => b.classList.toggle("on", b.dataset.when === "none"));
      }
      updateStepBars();
    });
  });

  $$("[data-when]").forEach((btn) => {
    btn.addEventListener("click", () => {
      createWhen = btn.dataset.when;
      $$("[data-when]").forEach((b) => b.classList.toggle("on", b === btn));
      updateStepBars();
    });
  });

  $("#create-title").addEventListener("input", updateStepBars);

  $("#create-private").addEventListener("change", (e) => {
    if (e.target.checked) {
      $("#create-share").checked = false;
      $("#create-share").disabled = true;
    } else {
      $("#create-share").disabled = false;
    }
  });

  $("#btn-create-save").addEventListener("click", () => {
    const title = $("#create-title").value.trim();
    if (!title) {
      $("#create-title").focus();
      return;
    }
    const body = $("#create-body").value.trim() || "（无正文）";
    const isPrivate = $("#create-private").checked;

    if (createKind === "memo") {
      const target = isPrivate ? privateMemos : memos;
      target.unshift({
        id: `new-${Date.now()}`,
        title,
        kind: "账号",
        sub: isPrivate ? "私密备忘 · 刚刚" : "备忘 · 刚刚",
        ico: title.slice(0, 1),
        updated: "刚刚",
        tags: isPrivate ? ["私密"] : ["新建"],
        fields: [],
        body,
      });
      renderMemos();
      renderPrivate();
      setView(isPrivate ? "private" : "memos");
      if (isPrivate) {
        $$("[data-private-tab]").forEach((b) =>
          b.classList.toggle("active", b.dataset.privateTab === "memos")
        );
        $("#private-care").hidden = true;
        $("#private-tasks").hidden = true;
        $("#private-memos").hidden = false;
      }
    } else {
      let due = null;
      let bucket = "inbox";
      if (createWhen === "today") {
        due = TODAY;
        bucket = "due";
      } else if (createWhen === "span") {
        due = "2026-11-25";
        bucket = "later";
      } else if (createWhen === "later" || createWhen === "none") {
        bucket = "inbox";
      }
      if (isPrivate) {
        privateTasks.unshift({
          id: `pt-${Date.now()}`,
          title,
          meta: due ? `私密 · ${due}` : "私密 · 未安排",
          body,
          done: false,
        });
        renderPrivate();
        setView("private");
        $$("[data-private-tab]").forEach((b) =>
          b.classList.toggle("active", b.dataset.privateTab === "tasks")
        );
        $("#private-care").hidden = true;
        $("#private-tasks").hidden = false;
        $("#private-memos").hidden = true;
      } else {
        tasks.unshift({
          id: `t-${Date.now()}`,
          title,
          due,
          bucket,
          tags: ["新建"],
          priority: "普通",
          category: "任务",
          private: false,
          visibility: "节点可见",
          version: 1,
          updated: "刚刚",
          time: null,
          body,
          done: false,
        });
        renderToday();
        renderCalendar();
        setView("today");
      }
    }
    closeDrawers();
  });

  $("#btn-edit-from-detail").addEventListener("click", () => {
    if (!lastDetail) return;
    openCreate();
    $("#create-title").value = lastDetail.title || "";
    $("#create-body").value = lastDetail.body || "";
    updateStepBars();
  });

  $("#btn-copy-from-detail").addEventListener("click", () => {
    if (!lastDetail) return;
    const lines = [lastDetail.title];
    (lastDetail.fields || []).forEach((f) => lines.push(`${f.lab}：${f.val}`));
    lines.push("", lastDetail.body || "");
    copyText(lines.join("\n"));
  });

  let remain = 2 * 60 + 47;
  setInterval(() => {
    remain = Math.max(0, remain - 1);
    const mm = String(Math.floor(remain / 60)).padStart(2, "0");
    const ss = String(remain % 60).padStart(2, "0");
    lockTimer.textContent = remain ? `${mm}:${ss} 后锁定` : "已到期（效果稿）";
  }, 1000);

  $("#today-date").textContent = fmtDateCN(new Date(2026, 10, 23));

  renderAll();
})();
