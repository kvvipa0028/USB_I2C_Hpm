use super::*;
use crate::i2c::parse::clock;
use crate::spi::micros_text;
use crate::theme;
use crate::widgets::{self, Table, Tone};

pub(super) fn page(ui: &mut egui::Ui, p: &mut Pwm, now: Duration) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("PWM 与频率捕获").size(20.0).strong());
        widgets::badge(ui, "本地演示 · 未接硬件", Tone::Neutral);
        let valid = p.validated_draft();
        let saved = p
            .saved
            .map(|(cfg, capture, compare, _)| (cfg, capture, compare));
        let current = valid
            .as_ref()
            .ok()
            .map(|cfg| (*cfg, p.capture.cfg, p.compare));
        if widgets::action_button(ui, current.is_some() && current != saved, "保存配置")
            .on_hover_text(crate::local::SAVE_HOVER)
            .on_disabled_hover_text("参数已保存，或填写有误；只保留在本窗口")
            .clicked()
        {
            p.save(now);
        }
        if widgets::quiet_button(ui, p.saved.is_some() && !p.capture.running, "恢复保存")
            .on_hover_text("恢复两侧配置与对比选项；只改输出草稿，不改变已应用输出")
            .on_disabled_hover_text("先保存配置；捕获演示运行时需先停止")
            .clicked()
        {
            p.restore_saved(now);
        }
        if widgets::action_button(ui, !p.log.is_empty(), "复制 CSV")
            .on_hover_text("复制本窗口的演示记录，包含时间、配置版本和示例来源；不是硬件测量")
            .clicked()
        {
            ui.ctx().copy_text(p.csv());
            p.copied_at = Some(now);
        }
    });
    widgets::muted(
        ui,
        "输出与捕获独立启停；对比使用已应用配置，不使用正在编辑的草稿。",
    );
    if let Some((_, _, _, at)) = p.saved {
        widgets::muted(ui, &format!("配置已保存 · {} · 仅本窗口有效", clock(at)));
    }
    if p.copied_at
        .is_some_and(|at| now.saturating_sub(at) < Duration::from_secs(3))
    {
        widgets::muted(ui, "演示记录已复制到剪贴板");
        ui.ctx().request_repaint_after(Duration::from_secs(3));
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("pwm_capture_page").auto_shrink([false, false]).show(ui, |ui| {
        notice(ui, p, now);
        ui.add_space(12.0);
        let left = ((ui.available_width() - 14.0) * 0.42).max(330.0);
        widgets::work_columns(ui, p, left, 830.0,
            |ui, p| output(ui, p, now),
            |ui, p| capture(ui, p, now));
        ui.add_space(10.0);
        widgets::note(ui, "回读接线示意：PWM OUT → 外部跳线 → CAP IN，并共地。独立引脚；映射、电平、量程及并发能力待确认，请勿按占位通道名直接接线。");
        ui.add_space(12.0);
        let left = (ui.available_width() - 14.0) * 0.52;
        widgets::work_columns(ui, p, left, 900.0,
            |ui, p| timing(ui, p),
            comparison);
        ui.add_space(12.0);
        let count = p.log.len();
        widgets::disclosure(ui, &mut p.show_log, "操作记录", count);
        if p.show_log { log(ui, p); }
        widgets::muted(ui, "离线演示，不发送 USB 命令。计时分辨率、频率范围、精度及输入耐压均未经硬件验证。");
    });
}

fn notice(ui: &mut egui::Ui, p: &mut Pwm, now: Duration) {
    let t = theme::of(ui);
    egui::Frame::new()
        .fill(t.pill)
        .corner_radius(7)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                egui::RichText::new("界面演示：所有捕获值均为固定示例，固件尚无 PWM / 捕获命令。")
                    .size(12.0),
            );
            ui.horizontal_wrapped(|ui| {
                widgets::muted(ui, "预览场景");
                let mut scene = p.scenario;
                ui.allocate_ui(egui::vec2(180.0, 26.0), |ui| {
                    widgets::pick(ui, &mut scene, &capture::SCENARIOS);
                });
                p.set_scenario(scene, now);
                widgets::muted(ui, "仅影响本页，不改变真实 USB 连接");
            });
        });
}

fn field(ui: &mut egui::Ui, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        widgets::muted(ui, label);
        body(ui);
    });
}

fn output(ui: &mut egui::Ui, p: &mut Pwm, now: Duration) {
    let status = match p.output {
        Output::Stopped => ("演示 · 已停止", Tone::Neutral),
        Output::Running => ("演示 · 输出中", Tone::Green),
        Output::Unknown => ("演示 · 状态未知", Tone::Amber),
    };
    widgets::panel(
        ui,
        "OUT   PWM 输出",
        |ui| {
            widgets::badge(ui, status.0, status.1);
        },
        |ui| {
            let top = ui.cursor().top();
            ui.columns(2, |cols| {
                field(&mut cols[0], "输出通道", |ui| {
                    widgets::pick(ui, &mut p.draft.channel, &CHANNELS);
                });
                field(&mut cols[1], "有效电平", |ui| {
                    widgets::pick(ui, &mut p.draft.active_high, &LEVELS);
                });
            });
            ui.columns(2, |cols| {
                field(&mut cols[0], "设定频率", |ui| {
                    ui.horizontal(|ui| {
                        widgets::unit_edit(ui, &mut p.hz, "10000", "Hz");
                    });
                });
                field(&mut cols[1], "有效占空比", |ui| {
                    ui.horizontal(|ui| {
                        widgets::unit_edit(ui, &mut p.duty, "40.0", "%");
                    });
                });
            });
            if let Ok(duty) = parse_duty(&p.duty) {
                p.draft.duty = duty;
            }
            let mut duty = p.draft.duty;
            if widgets::slider(ui, &mut duty, 0.0..=100.0, 0.1).changed() {
                p.draft.duty = duty;
                p.duty = format!("{duty:.1}");
            }
            widgets::table_foot(ui, "0%", "100% · 修改后点应用修改");
            match p.validated_draft() {
                Ok(cfg) => {
                    p.draft = cfg;
                    ui.horizontal_wrapped(|ui| {
                        widgets::muted(ui, &format!("草稿周期 {}", micros_text(cfg.period_us())));
                        widgets::muted(ui, &format!("高电平 {}", micros_text(cfg.high_us())));
                    });
                }
                Err(e) => widgets::error(ui, &e),
            }
            widgets::disclosure(ui, &mut p.advanced, "高级参数", 3);
            if p.advanced {
                widgets::field_row(ui, "停止状态", false, |ui| {
                    widgets::pick(ui, &mut p.draft.idle, &IDLES);
                });
                widgets::field_row(ui, "对齐方式", false, |ui| {
                    widgets::pick(ui, &mut p.draft.center, &ALIGNS);
                });
                widgets::field_row(ui, "启动延时", true, |ui| {
                    widgets::unit_edit(ui, &mut p.delay, "0", "ms");
                });
                widgets::muted(
                    ui,
                    "停止状态和延时只记入配置，不模拟引脚或延时。选项均待硬件确认。",
                );
            }
            let applied = match p.applied {
                Some(a) => format!(
                    "已应用 v{} · PWM{} · {} · {:.1}% 有效",
                    a.revision,
                    a.cfg.channel,
                    hz_text(f64::from(a.cfg.hz)),
                    a.cfg.duty
                ),
                None => "尚未应用；首次启动演示会应用当前有效配置".into(),
            };
            widgets::muted(ui, &applied);
            if p.applied.is_some() && p.dirty() {
                widgets::muted(ui, "有未应用修改；输出及对比仍使用上方已应用配置");
            }
            ui.add_space((267.0 - (ui.cursor().top() - top)).max(0.0));
            widgets::hairline(ui);
            let valid = p.validated_draft().is_ok();
            let allowed = p.scenario != Scenario::Interrupted;
            ui.horizontal_wrapped(|ui| {
                if widgets::action_button(ui, allowed && valid && p.dirty(), "应用修改")
                    .on_hover_text("只更新本地已应用快照，不发送 USB；启动和停止仍独立控制")
                    .clicked()
                {
                    p.apply(now);
                }
                if p.output == Output::Running {
                    if widgets::ink_button(ui, true, "停止输出 · 演示").clicked() {
                        p.stop_output(now);
                    }
                } else if widgets::ink_button(
                    ui,
                    allowed && (p.applied.is_some() || valid),
                    "启动输出 · 演示",
                )
                .clicked()
                {
                    p.start_output(now);
                }
            });
            ui.horizontal_wrapped(|ui| {
                if widgets::quiet_button(ui, p.dirty(), "撤销修改").clicked() {
                    p.revert();
                }
                if widgets::quiet_button(ui, true, "恢复默认")
                    .on_hover_text("只重置草稿，不改变正在运行的本地演示或已应用配置")
                    .clicked()
                {
                    p.draft = Config::default();
                    p.sync_text();
                }
            });
        },
    );
}

fn capture(ui: &mut egui::Ui, p: &mut Pwm, now: Duration) {
    let status = if p.scenario == Scenario::Interrupted {
        ("演示 · 连接中断", Tone::Amber)
    } else if !p.capture.running {
        ("演示 · 已停止", Tone::Neutral)
    } else if p.scenario == Scenario::NoSignal {
        ("演示 · 无有效边沿", Tone::Amber)
    } else if p.capture.latest.is_none() {
        ("演示 · 等待统计窗口", Tone::Neutral)
    } else {
        ("演示 · 捕获中", Tone::Green)
    };
    widgets::panel(
        ui,
        "IN   频率捕获",
        |ui| {
            widgets::badge(ui, status.0, status.1);
        },
        |ui| {
            let top = ui.cursor().top();
            let before = p.capture.cfg;
            ui.add_enabled_ui(!p.capture.running, |ui| {
                ui.columns(3, |cols| {
                    field(&mut cols[0], "输入通道", |ui| {
                        widgets::pick(ui, &mut p.capture.cfg.channel, &capture::CHANNELS);
                    });
                    field(&mut cols[1], "示例项目", |ui| {
                        widgets::pick(ui, &mut p.capture.cfg.mode, &capture::MODES);
                    });
                    field(&mut cols[2], "统计窗口 · 演示", |ui| {
                        widgets::pick(ui, &mut p.capture.cfg.window_ms, &capture::WINDOWS);
                    });
                });
            })
            .response
            .on_disabled_hover_text("先停止捕获演示，再修改输入配置");
            if before != p.capture.cfg {
                p.clear_capture(now);
            }
            ui.add_space(6.0);
            widgets::muted(ui, "捕获频率 / 示例数据");
            let reading = p.capture.latest;
            let frequency = reading.map_or_else(|| "—".into(), |r| format!("{:.3}", r.hz / 1000.0));
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(frequency).monospace().size(38.0));
                widgets::muted(ui, "kHz");
                if let Some(c) = p.comparison() {
                    widgets::muted(ui, &format!("{:+.0} ppm · 示例偏差", c.ppm));
                }
            });
            widgets::hairline(ui);
            ui.columns(2, |cols| {
                metric(
                    &mut cols[0],
                    "示例周期",
                    reading.map(|r| micros_text(r.period_us())),
                );
                metric(
                    &mut cols[1],
                    "高电平占空比",
                    reading
                        .and_then(|r| r.high_duty)
                        .map(|d| format!("{d:.2}%")),
                );
            });
            let times = reading.and_then(|r| {
                r.high_us().map(|h| {
                    format!(
                        "tH {}    tL {}",
                        micros_text(h),
                        micros_text(r.period_us() - h)
                    )
                })
            });
            widgets::muted(ui, times.as_deref().unwrap_or("tH —    tL —"));
            widgets::muted(
                ui,
                if p.capture.cfg.mode == capture::Mode::Frequency {
                    "仅频率：不提供占空比与高低电平时序"
                } else {
                    "占空比需要双边沿捕获；当前固件尚不支持"
                },
            );
            let info = match reading {
                Some(r) => format!("有效示例 {} 窗 · 最近 {}", p.capture.samples, clock(r.at)),
                None => format!("有效示例 {} 窗 · 当前无有效结果", p.capture.samples),
            };
            widgets::muted(ui, &info);
            ui.add_space((267.0 - (ui.cursor().top() - top)).max(0.0));
            widgets::hairline(ui);
            ui.horizontal_wrapped(|ui| {
                if widgets::quiet_button(
                    ui,
                    p.capture.samples > 0 || p.capture.latest.is_some(),
                    "清空统计",
                )
                .clicked()
                {
                    p.clear_capture(now);
                }
                if p.capture.running {
                    if widgets::ink_button(ui, true, "停止捕获 · 演示").clicked() {
                        p.stop_capture(now);
                    }
                } else if widgets::ink_button(
                    ui,
                    p.scenario != Scenario::Interrupted,
                    "开始捕获 · 演示",
                )
                .clicked()
                {
                    p.start_capture(now);
                }
            });
            widgets::muted(ui, "独立示例源；停止输出不停止捕获，也不模拟电气回读。");
            if p.scenario == Scenario::Interrupted {
                widgets::muted(ui, "数据已失效；切回正常场景后需分别手动启动。");
            }
        },
    );
}

fn metric(ui: &mut egui::Ui, label: &str, value: Option<String>) {
    widgets::muted(ui, label);
    ui.label(
        egui::RichText::new(value.unwrap_or_else(|| "—".into()))
            .monospace()
            .size(22.0),
    );
}

fn comparison(ui: &mut egui::Ui, p: &mut Pwm) {
    widgets::panel(
        ui,
        "设定 / 捕获示例",
        |_| {},
        |ui| {
            widgets::check(ui, &mut p.compare, "对比已应用配置 · 示例");
            let applied = p.applied.map(|a| a.cfg);
            let reading = p.capture.latest;
            let compare = p.comparison();
            let cfg_hz = applied.map(|c| hz_text(f64::from(c.hz)));
            let cap_hz = reading.map(|r| hz_text(r.hz));
            let widths = [
                84.0,
                (ui.available_width() - 84.0) / 3.0,
                (ui.available_width() - 84.0) / 3.0,
                0.0,
            ];
            let table = Table { widths: &widths };
            table.header(ui, &["指标", "已应用设定", "捕获示例", "偏差"]);
            let rows = [
                (
                    "频率",
                    cfg_hz,
                    cap_hz,
                    compare.as_ref().map(|c| format!("{:+.3} Hz", c.hz)),
                ),
                (
                    "周期",
                    applied.map(|c| micros_text(c.period_us())),
                    reading.map(|r| micros_text(r.period_us())),
                    compare.as_ref().map(|c| format!("{:+.3} µs", c.period_us)),
                ),
                (
                    "高电平占空比",
                    applied.map(|c| format!("{:.2}%", c.high_duty())),
                    reading
                        .and_then(|r| r.high_duty)
                        .map(|d| format!("{d:.2}%")),
                    compare
                        .and_then(|c| c.duty_pp)
                        .map(|d| format!("{d:+.2} pp")),
                ),
            ];
            let t = theme::of(ui);
            for (label, set, value, delta) in rows {
                let shown = |s: Option<String>| s.unwrap_or_else(|| "—".into());
                table.row(
                    ui,
                    &[
                        (label.into(), t.fg2, false),
                        (shown(set), t.fg, true),
                        (shown(value), t.fg, true),
                        (shown(delta), t.fg2, true),
                    ],
                    false,
                );
            }
            ui.add_space(6.0);
            if let Some(a) = p.applied {
                widgets::muted(
                    ui,
                    &format!(
                        "参考 PWM{} · 已应用 v{}；输入 CAP{}",
                        a.cfg.channel, a.revision, p.capture.cfg.channel
                    ),
                );
            }
            widgets::muted(ui, "低电平有效会换算为高电平占空比；pp 为百分点。");
            widgets::muted(
                ui,
                "停机、中断、无有效数据、0% / 100% 或等待新版配置样本时不计算偏差。",
            );
            widgets::muted(ui, "这是示例数值对比，不证明两通道已连接或硬件回读通过。");
        },
    );
}

/// Parameter reconstruction on one shared time scale. A density cap keeps extreme
/// frequency ratios bounded and avoids rendering aliased edges as real detail.
pub(super) fn trace(
    hz: f64,
    duty: f64,
    active_high: bool,
    center: bool,
    span_us: f64,
) -> Option<Vec<(f32, bool)>> {
    if !hz.is_finite()
        || hz <= 0.0
        || !span_us.is_finite()
        || span_us <= 0.0
        || !duty.is_finite()
        || !(0.0..=100.0).contains(&duty)
    {
        return None;
    }
    let period = 1e6 / hz;
    let cycles = span_us / period;
    if duty == 0.0 || duty == 100.0 {
        let level = (duty == 100.0) == active_high;
        return Some(vec![(0.0, level), (1.0, level)]);
    }
    if cycles > 128.0 {
        return None;
    }
    let d = duty / 100.0;
    let a = if center { (1.0 - d) / 2.0 } else { 0.0 };
    let b = a + d;
    let mut points = Vec::new();
    for i in 0..cycles.ceil() as usize {
        for (from, to, active) in [(0.0, a, false), (a, b, true), (b, 1.0, false)] {
            let from = (i as f64 + from) / cycles;
            let to = ((i as f64 + to) / cycles).min(1.0);
            if to > from && from < 1.0 {
                points.extend([
                    (from as f32, active == active_high),
                    (to as f32, active == active_high),
                ]);
            }
        }
    }
    Some(points)
}

fn timing(ui: &mut egui::Ui, p: &Pwm) {
    widgets::panel(
        ui,
        "时序对照",
        |ui| {
            widgets::badge(ui, "参数示意 · 非采样", Tone::Neutral);
        },
        |ui| {
            let applied = p.applied.map(|a| a.cfg);
            let reading = p.capture.latest;
            let period = applied
                .map(|c| c.period_us())
                .or_else(|| reading.map(|r| r.period_us()));
            let span = period.unwrap_or(100.0) * 4.0;
            let t = theme::of(ui);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), 174.0),
                egui::Sense::hover(),
            );
            let painter = ui.painter_at(rect);
            let left = rect.left() + 66.0;
            let right = rect.right() - 16.0;
            let width = (right - left).max(1.0);
            let small = egui::FontId::proportional(10.5);
            for i in 0..=4 {
                let x = left + width * i as f32 / 4.0;
                painter.vline(
                    x,
                    rect.top() + 5.0..=rect.bottom() - 26.0,
                    egui::Stroke::new(1.0, t.line2),
                );
                painter.text(
                    egui::pos2(x, rect.bottom() - 12.0),
                    egui::Align2::CENTER_CENTER,
                    trim(span * f64::from(i) / 4.0, 2),
                    egui::FontId::monospace(10.0),
                    t.muted,
                );
            }
            let out = applied.and_then(|c| {
                trace(
                    f64::from(c.hz),
                    f64::from(c.duty),
                    c.active_high,
                    c.center,
                    span,
                )
            });
            let cap =
                reading.and_then(|r| r.high_duty.and_then(|d| trace(r.hz, d, true, false, span)));
            for (row, label, channel, points, color, empty) in [
                (
                    0,
                    "输出设定",
                    applied.map_or_else(|| "未应用".into(), |c| format!("PWM{}", c.channel)),
                    out,
                    t.accent,
                    "尚未应用配置",
                ),
                (
                    1,
                    "捕获示例",
                    format!("CAP{}", p.capture.cfg.channel),
                    cap,
                    t.fg2,
                    if reading.is_some() {
                        "仅频率或时序过密，省略示意"
                    } else {
                        "当前无有效示例"
                    },
                ),
            ] {
                let high = rect.top() + 12.0 + row as f32 * 74.0;
                let low = high + 34.0;
                for y in [high, low] {
                    painter.hline(left..=right, y, egui::Stroke::new(1.0, t.line2));
                }
                painter.text(
                    egui::pos2(rect.left(), high + 8.0),
                    egui::Align2::LEFT_CENTER,
                    label,
                    small.clone(),
                    t.muted,
                );
                painter.text(
                    egui::pos2(rect.left(), low - 6.0),
                    egui::Align2::LEFT_CENTER,
                    channel,
                    small.clone(),
                    t.muted,
                );
                if let Some(points) = points {
                    painter.add(egui::Shape::line(
                        points
                            .into_iter()
                            .map(|(x, h)| egui::pos2(left + x * width, if h { high } else { low }))
                            .collect(),
                        egui::Stroke::new(1.8, color),
                    ));
                } else {
                    painter.text(
                        egui::pos2(left + 6.0, (high + low) / 2.0),
                        egui::Align2::LEFT_CENTER,
                        empty,
                        small.clone(),
                        t.muted,
                    );
                }
            }
            widgets::muted(ui, "横轴 µs；两行独立对齐，不表示相位或传播延时。");
            widgets::muted(ui, "上行为已应用配置示意，停止后保留，不代表引脚电平。");
        },
    );
}

fn log(ui: &mut egui::Ui, p: &Pwm) {
    let t = theme::of(ui);
    let table = Table {
        widths: &[72.0, 146.0, 110.0, 0.0],
    };
    table.header(ui, &["时间", "事件 · 本地演示", "已应用参考", "捕获示例"]);
    egui::ScrollArea::vertical()
        .id_salt("pwm_records")
        .max_height(220.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for r in &p.log {
                let reference = r.applied.map_or_else(
                    || "—".into(),
                    |a| format!("PWM{} · v{}", a.cfg.channel, a.revision),
                );
                let reading = r.reading.map_or_else(
                    || "—".into(),
                    |v| format!("CAP{} · {}", r.capture.channel, hz_text(v.hz)),
                );
                table.row(
                    ui,
                    &[
                        (clock(r.at), t.muted, true),
                        (r.event.into(), t.fg2, false),
                        (reference, t.fg, true),
                        (reading, t.fg, true),
                    ],
                    false,
                );
            }
        });
    widgets::muted(ui, &format!("最新在上，最多 {LOG_MAX} 条；复制 CSV 为时间正序，保留每条记录当时的配置。保存和记录仅在本窗口有效。"));
}
