use super::*;
use capture::Mode;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn running() -> Pwm {
    let mut p = Pwm {
        compare: true,
        ..Pwm::default()
    };
    p.start_output(ms(0));
    p.start_capture(ms(0));
    p.tick(ms(100));
    p
}

#[test]
fn starts_empty_and_capture_is_an_independent_example_source() {
    let mut p = Pwm::default();
    assert!(p.applied.is_none() && p.capture.latest.is_none() && p.log.is_empty());
    p.start_capture(ms(0));
    assert!(p.tick(ms(99)));
    assert!(p.capture.latest.is_none());
    assert!(p.tick(ms(100)));
    assert_eq!(p.capture.latest.unwrap().hz, 9998.0);
    assert_eq!(p.output, Output::Stopped);
    assert!(p.comparison().is_none());
}

#[test]
fn comparison_uses_applied_not_draft_and_waits_for_new_revision() {
    let mut p = running();
    let c = p.comparison().unwrap();
    assert!((c.hz + 2.0).abs() < 1e-9);
    assert!((c.ppm + 200.0).abs() < 1e-9);
    assert!((c.duty_pp.unwrap() + 0.02).abs() < 1e-6);
    p.hz = "20000".into();
    p.duty = "60".into();
    assert!(p.dirty());
    assert_eq!(p.comparison().unwrap().hz, -2.0);
    p.apply(ms(150));
    assert!(p.comparison().is_none());
    p.tick(ms(200));
    assert_eq!(p.comparison().unwrap().hz, -10002.0);
    assert_eq!(p.applied.unwrap().revision, 2);
}

#[test]
fn low_active_conversion_and_constant_output_suppress_edge_comparison() {
    let mut p = running();
    p.draft.active_high = false;
    p.apply(ms(150));
    p.tick(ms(200));
    assert!((p.comparison().unwrap().duty_pp.unwrap() + 20.02).abs() < 1e-6);
    for duty in ["0", "100"] {
        p.duty = duty.into();
        p.apply(ms(250));
        p.tick(ms(400));
        assert!(p.comparison().is_none());
        assert!(p.capture.latest.is_some()); // Independent source is not a loopback.
    }
}

#[test]
fn stop_output_leaves_capture_running_and_stop_capture_leaves_output_running() {
    let mut p = running();
    p.stop_output(ms(150));
    assert!(p.capture.running);
    assert!(p.comparison().is_none());
    p.tick(ms(200));
    assert_eq!(p.capture.samples, 2);
    p.start_output(ms(210));
    p.stop_capture(ms(220));
    assert_eq!(p.output, Output::Running);
    assert!(p.capture.latest.is_none());
    assert!(!p.tick(ms(5000))); // Output preview alone needs no periodic repaint.
    assert!(p.running());
}

#[test]
fn repeated_starts_and_stops_are_idempotent_and_use_applied_snapshot() {
    let mut p = running();
    let n = p.log.len();
    p.start_output(ms(110));
    p.start_capture(ms(110));
    assert_eq!(p.log.len(), n);
    assert_eq!(p.capture.samples, 1);
    p.stop_output(ms(120));
    p.hz = "2000".into();
    p.start_output(ms(130));
    assert_eq!(p.applied.unwrap().cfg.hz, 10000);
    p.stop_output(ms(140));
    p.stop_capture(ms(140));
    let n = p.log.len();
    p.stop_output(ms(150));
    p.stop_capture(ms(150));
    assert_eq!(p.log.len(), n);
}

#[test]
fn no_signal_invalidates_immediately_and_normal_waits_for_window() {
    let mut p = running();
    p.set_scenario(Scenario::NoSignal, ms(110));
    assert!(p.capture.latest.is_none() && p.comparison().is_none());
    p.tick(ms(5000));
    assert_eq!(p.capture.samples, 1);
    p.set_scenario(Scenario::Signal, ms(5100));
    p.tick(ms(5199));
    assert!(p.capture.latest.is_none());
    p.tick(ms(5200));
    assert!(p.capture.latest.is_some());
}

#[test]
fn interruption_invalidates_and_never_automatically_resumes() {
    let mut p = running();
    p.set_scenario(Scenario::Interrupted, ms(110));
    assert_eq!(p.output, Output::Unknown);
    assert!(!p.capture.running && p.capture.latest.is_none());
    p.hz = "1234".into();
    assert!(!p.apply(ms(120)));
    p.start_output(ms(120));
    p.start_capture(ms(120));
    assert!(!p.running());
    p.set_scenario(Scenario::Signal, ms(130));
    assert_eq!(p.output, Output::Unknown);
    assert!(!p.running());
    p.start_capture(ms(150));
    p.start_output(ms(150));
    p.tick(ms(250));
    assert!(p.comparison().is_some());
    assert_eq!(p.applied.unwrap().cfg.hz, 10000);
}

#[test]
fn frequency_only_omits_duty_pulse_width_and_duty_delta() {
    let mut p = Pwm::default();
    p.capture.cfg.mode = Mode::Frequency;
    p.compare = true;
    p.start_output(ms(0));
    p.start_capture(ms(0));
    p.tick(ms(100));
    let r = p.capture.latest.unwrap();
    assert!(r.high_duty.is_none() && r.high_us().is_none());
    assert!(p.comparison().unwrap().duty_pp.is_none());
    assert!(p.csv().contains("frequency_only"));
}

#[test]
fn clear_resets_samples_and_sample_records_without_affecting_output_or_session() {
    let mut p = running();
    p.clear_capture(ms(150));
    assert_eq!(p.output, Output::Running);
    assert!(p.capture.running);
    assert_eq!(p.capture.samples, 0);
    assert!(p.capture.latest.is_none());
    assert!(p.log.iter().all(|r| r.reading.is_none()));
    p.tick(ms(249));
    assert!(p.capture.latest.is_none());
    p.tick(ms(250));
    assert_eq!(p.capture.samples, 1);
}

#[test]
fn long_pause_skips_backlog_and_records_stay_bounded() {
    let mut p = running();
    p.tick(Duration::from_secs(86_400));
    assert_eq!(p.capture.samples, 2);
    assert_eq!(p.capture.latest.unwrap().at, Duration::from_secs(86_400));
    p.tick(ms(50)); // Non-monotonic input cannot underflow or generate a sample.
    assert_eq!(p.capture.samples, 2);
    for n in 1..=500 {
        p.tick(Duration::from_secs(86_400) + ms(n * 100));
    }
    assert_eq!(p.log.len(), LOG_MAX);
}

#[test]
fn csv_keeps_provenance_timestamp_configuration_revision_and_no_stale_measurements() {
    let mut p = running();
    p.hz = "20000".into();
    p.apply(ms(150));
    p.tick(ms(200));
    p.stop_capture(ms(250));
    let csv = p.csv();
    let lines: Vec<_> = csv.lines().collect();
    let cols = lines[0].split(',').count();
    assert_eq!(cols, 20);
    for line in &lines[1..] {
        assert!(line.starts_with("local_demo,"));
        assert_eq!(line.split(',').count(), cols);
    }
    assert!(lines
        .iter()
        .any(|l| l.contains("0.100000,捕获样本 · 演示,1,PWM0,10000,")));
    assert!(lines
        .iter()
        .any(|l| l.contains("0.200000,捕获样本 · 演示,2,PWM0,20000,")));
    assert!(lines.last().unwrap().ends_with(",,"));
}

#[test]
fn invalid_text_blocks_apply_even_when_advanced_is_hidden_and_revert_recovers() {
    let mut p = Pwm::default();
    for invalid in ["0", "1000001", "1.2", "NaN", "inf", "-1", ""] {
        p.hz = invalid.into();
        assert!(p.validated_draft().is_err());
        assert!(!p.apply(ms(0)));
    }
    p.revert();
    for invalid in ["NaN", "inf", "101", "-1", "12x"] {
        p.duty = invalid.into();
        assert!(!p.apply(ms(0)));
    }
    p.revert();
    p.delay = "60001".into();
    p.advanced = false;
    assert!(!p.apply(ms(0)));
    p.start_output(ms(0));
    assert_eq!(p.output, Output::Stopped);
    p.revert();
    assert!(p.apply(ms(0)));
    assert_eq!(parse_duty("12.34%"), Ok(12.3));
    assert_eq!(parse_hz("1000000"), Ok(PREVIEW_MAX_HZ));
}

#[test]
fn timing_trace_handles_polarity_alignment_extremes_and_shared_scale() {
    assert_eq!(
        ui::trace(10000.0, 25.0, true, false, 100.0).unwrap(),
        vec![(0.0, true), (0.25, true), (0.25, false), (1.0, false)]
    );
    let low = ui::trace(10000.0, 25.0, false, false, 100.0).unwrap();
    assert_eq!(low[0], (0.0, false));
    let center = ui::trace(10000.0, 50.0, true, true, 100.0).unwrap();
    assert_eq!(center[2], (0.25, true));
    assert_eq!(center[3], (0.75, true));
    assert_eq!(
        ui::trace(10000.0, 0.0, true, false, 400.0).unwrap(),
        vec![(0.0, false), (1.0, false)]
    );
    assert_eq!(
        ui::trace(10000.0, 100.0, false, false, 400.0).unwrap(),
        vec![(0.0, false), (1.0, false)]
    );
    let tr = ui::trace(9998.0, 39.98, true, false, 400.0).unwrap();
    assert_eq!(tr.last().unwrap().0, 1.0);
    assert!(tr
        .iter()
        .all(|(x, _)| x.is_finite() && (0.0..=1.0).contains(x)));
    assert!(ui::trace(1e6, 50.0, true, false, 4e6).is_none());
    for hz in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(ui::trace(hz, 50.0, true, false, 400.0).is_none());
    }
}

struct Harness {
    ctx: egui::Context,
    time: f64,
    texts: Vec<(String, egui::Pos2)>,
}
impl Harness {
    fn new(p: &mut Pwm) -> Self {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut h = Self {
            ctx,
            time: 0.0,
            texts: Vec::new(),
        };
        h.frame(p, vec![]);
        h
    }
    fn frame(&mut self, p: &mut Pwm, events: Vec<egui::Event>) {
        self.time += 0.02;
        let now = Duration::from_secs_f64(self.time);
        p.tick(now);
        let mut out = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 1300.0),
                )),
                time: Some(self.time),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| page(ui, p, now));
            },
        );
        out.textures_delta.clear();
        self.texts.clear();
        for shape in out.shapes {
            self.collect(&shape.shape, shape.clip_rect);
        }
    }
    fn collect(&mut self, shape: &egui::Shape, clip: egui::Rect) {
        match shape {
            egui::Shape::Text(t) => {
                let pos = t.pos + t.galley.rect.center().to_vec2();
                if clip.contains(pos) {
                    self.texts.push((t.galley.text().into(), pos));
                }
            }
            egui::Shape::Vec(v) => {
                for s in v {
                    self.collect(s, clip);
                }
            }
            _ => {}
        }
    }
    fn click(&mut self, p: &mut Pwm, label: &str) {
        let pos = self
            .texts
            .iter()
            .rev()
            .find(|(t, _)| t == label)
            .unwrap_or_else(|| panic!("missing {label:?}: {:?}", self.texts))
            .1;
        self.frame(p, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(
                p,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
            );
        }
        self.frame(p, vec![]);
    }
    fn replace(&mut self, p: &mut Pwm, before: &str, after: &str) {
        self.click(p, before);
        self.frame(
            p,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    },
                },
                egui::Event::Text(after.into()),
            ],
        );
        self.frame(p, vec![]);
    }
}

#[test]
fn pointer_independent_start_stop_and_clear() {
    let mut p = Pwm::default();
    let mut h = Harness::new(&mut p);
    h.click(&mut p, "启动输出 · 演示");
    assert_eq!(p.output, Output::Running);
    h.click(&mut p, "开始捕获 · 演示");
    assert!(p.capture.running);
    for _ in 0..6 {
        h.frame(&mut p, vec![]);
    }
    assert!(p.capture.latest.is_some());
    h.click(&mut p, "停止输出 · 演示");
    assert_eq!(p.output, Output::Stopped);
    assert!(p.capture.running);
    h.click(&mut p, "清空统计");
    assert!(p.capture.running);
    h.click(&mut p, "停止捕获 · 演示");
    assert!(!p.capture.running && p.capture.latest.is_none());
}

#[test]
fn pointer_invalid_frequency_survives_blur_and_cannot_be_applied() {
    let mut p = Pwm::default();
    let mut h = Harness::new(&mut p);
    h.replace(&mut p, "10000", "0");
    h.click(&mut p, "应用修改");
    assert_eq!(p.hz, "0");
    assert!(p.applied.is_none());
    h.click(&mut p, "启动输出 · 演示");
    assert_eq!(p.output, Output::Stopped);
    h.click(&mut p, "撤销修改");
    assert_eq!(p.hz, "10000");
    h.click(&mut p, "启动输出 · 演示");
    assert_eq!(p.output, Output::Running);
}

#[test]
fn pointer_capture_mode_stays_locked_until_stopped() {
    let mut p = Pwm::default();
    let mut h = Harness::new(&mut p);
    h.click(&mut p, "开始捕获 · 演示");
    h.click(&mut p, "频率 + 占空比");
    assert!(!h.texts.iter().any(|(t, _)| t == "仅频率"));
    h.click(&mut p, "停止捕获 · 演示");
    h.click(&mut p, "频率 + 占空比");
    h.click(&mut p, "仅频率");
    assert_eq!(p.capture.cfg.mode, Mode::Frequency);
    // Close the menu on an outside click before beginning the new session.
    h.click(&mut p, "IN   频率捕获");
    h.click(&mut p, "开始捕获 · 演示");
    for _ in 0..6 {
        h.frame(&mut p, vec![]);
    }
    assert!(p.capture.latest.unwrap().high_duty.is_none());
}

#[test]
fn combined_page_renders_all_states_both_themes_and_supported_sizes() {
    for dark in [false, true] {
        for (w, h) in [(900.0, 600.0), (1200.0, 800.0), (1600.0, 1000.0)] {
            for state in 0..6 {
                let ctx = egui::Context::default();
                crate::fonts::install(&ctx);
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut p = running();
                match state {
                    0 => p = Pwm::default(),
                    1 => {}
                    2 => p.set_scenario(Scenario::NoSignal, ms(200)),
                    3 => p.set_scenario(Scenario::Interrupted, ms(200)),
                    4 => {
                        p.stop_capture(ms(200));
                        p.capture.cfg.mode = Mode::Frequency;
                        p.start_capture(ms(200));
                        p.tick(ms(300));
                    }
                    5 => {
                        p.advanced = true;
                        p.hz = "NaN".into();
                        p.delay = "bad".into();
                        p.show_log = true;
                    }
                    _ => unreachable!(),
                }
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(w, h),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::Panel::left("nav")
                            .exact_size(crate::nav::WIDTH)
                            .show(ui, |_| {});
                        egui::CentralPanel::default().show(ui, |ui| page(ui, &mut p, ms(300)));
                    },
                );
                output.textures_delta.clear();
                for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
                    assert!(primitive.clip_rect.is_finite());
                    if let egui::epaint::Primitive::Mesh(m) = primitive.primitive {
                        assert!(m.vertices.iter().all(|v| v.pos.is_finite()));
                    }
                }
            }
        }
    }
}

#[test]
fn saved_configuration_restores_both_sides_but_never_reconfigures_active_capture() {
    let mut p = running();
    p.hz = "1234".into();
    assert!(p.save(ms(150)));
    p.hz = "NaN".into();
    assert!(!p.save(ms(160)));
    assert!(!p.restore_saved(ms(170)));
    p.stop_capture(ms(180));
    p.capture.cfg.channel = 1;
    p.compare = false;
    assert!(p.restore_saved(ms(190)));
    assert_eq!(p.hz, "1234");
    assert_eq!(p.capture.cfg.channel, 0);
    assert!(p.compare);
    assert_eq!(p.output, Output::Running);
    assert_eq!(p.applied.unwrap().cfg.hz, 10000);
    let h = Harness::new(&mut p);
    assert!(h
        .texts
        .iter()
        .any(|(text, _)| text.starts_with("配置已保存 · 00:00.150")));
}
