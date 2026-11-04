use super::*;

#[test]
fn payload_lengths_follow_classic_and_fd_dlc_encoding() {
    let fd_lengths = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];
    for protocol in [Protocol::Classic, Protocol::Fd] {
        let mut c = Can::default();
        c.cfg.protocol = protocol;
        for len in 0..=65 {
            c.send_data = "A5 ".repeat(len);
            let expected = match protocol {
                Protocol::Classic => (len <= 8).then_some(len as u8),
                Protocol::Fd => fd_lengths.iter().position(|&n| n == len).map(|i| i as u8),
            };
            let before = (c.tx, c.log.len());
            let result = c.send(Duration::ZERO);
            match expected {
                Some(dlc) => {
                    result.unwrap();
                    let f = c.log.back().unwrap();
                    assert_eq!(f.dlc, dlc, "{protocol:?}, length={len}");
                    assert_eq!(f.data, vec![0xa5; len]);
                    assert_eq!(f.protocol, protocol);
                    assert_eq!(f.brs, protocol == Protocol::Fd);
                }
                None => {
                    assert!(result.is_err(), "{protocol:?}, length={len}");
                    assert_eq!((c.tx, c.log.len()), before);
                    assert_eq!(c.send_data, "A5 ".repeat(len));
                }
            }
        }
    }
}

#[test]
fn both_protocols_check_standard_extended_ids_and_listen_only() {
    for protocol in [Protocol::Classic, Protocol::Fd] {
        let mut c = Can::default();
        c.cfg.protocol = protocol;
        for (ext, valid, invalid) in [
            (false, "0x7FF", "0x800"),
            (true, "0x1FFFFFFF", "0x20000000"),
        ] {
            c.cfg.ext = ext;
            c.send_id = valid.into();
            c.send(Duration::ZERO).unwrap();
            assert_eq!(c.log.back().unwrap().ext, ext);
            let before = c.tx;
            c.send_id = invalid.into();
            assert!(c.send(Duration::ZERO).is_err());
            assert_eq!(c.tx, before);
        }
        c.send_id = "0x300".into();
        c.send_data = "GG".into();
        assert!(c.send(Duration::ZERO).is_err());
        c.send_data = "00".into();
        c.cfg.listen_only = true;
        let before = c.tx;
        assert_eq!(c.send(Duration::ZERO).unwrap_err(), "只听模式不发送");
        assert_eq!(c.tx, before);
    }
}

#[test]
fn frame_history_keeps_protocol_brs_and_separate_intervals() {
    let mut c = Can::default();
    c.cfg.protocol = Protocol::Fd;
    c.send_data = "A5 ".repeat(64);
    c.send(Duration::from_millis(200)).unwrap();
    let fd_n = c.next_n - 1;
    let f = c.log.back().unwrap();
    assert_eq!((f.protocol, f.brs, f.dlc), (Protocol::Fd, true, 15));
    assert_eq!(f.gap, None); // The classic seed with the same ID is a different stream.
    c.cfg.brs = false;
    c.send(Duration::from_millis(300)).unwrap();
    assert_eq!(c.log.back().unwrap().protocol_label(), "FD");
    assert_eq!(c.log.back().unwrap().gap, Some(Duration::from_millis(100)));
    c.cfg.protocol = Protocol::Classic;
    c.cfg.brs = true; // Remembered FD setting must not leak into classic frames.
    c.send_data = "00".into();
    c.send(Duration::from_millis(400)).unwrap();
    assert_eq!(c.log.back().unwrap().protocol_label(), "CAN");
    assert!(!c.log.back().unwrap().brs);
    let fd = c.log.iter().find(|f| f.n == fd_n).unwrap();
    assert_eq!((fd.protocol, fd.brs, fd.dlc), (Protocol::Fd, true, 15));
    assert_eq!(fd.data, vec![0xa5; 64]);
    assert!(c
        .log
        .iter()
        .take(8)
        .all(|f| f.protocol == Protocol::Classic));
    c.cfg.ext = true;
    c.send(Duration::from_millis(500)).unwrap();
    assert_eq!(c.log.back().unwrap().gap, None);
}

#[test]
fn fd_generator_uses_valid_long_payloads_filter_and_bounded_log() {
    let mut c = Can {
        receiving: true,
        ..Can::default()
    };
    c.cfg.protocol = Protocol::Fd;
    for i in 1..=20 {
        c.tick(Duration::from_millis(50 * i));
    }
    for (id, len, dlc, ext) in [
        (0x180, 12, 9, false),
        (0x280, 32, 13, false),
        (0x18ff_50e5, 64, 15, true),
    ] {
        let f = c.log.iter().rev().find(|f| f.id == id).unwrap();
        assert_eq!((f.protocol, f.brs, f.ext), (Protocol::Fd, true, ext));
        assert_eq!((f.data.len(), f.dlc), (len, dlc));
    }
    c.cfg.filter_id = "0x280".into();
    c.cfg.filter_mask = "0x1FFFFFFF".into();
    c.cfg.brs = false;
    let first = c.next_n;
    for i in 21..=2200 {
        c.tick(Duration::from_millis(50 * i));
    }
    assert_eq!(c.log.len(), LOG_MAX);
    assert!(c
        .log
        .iter()
        .filter(|f| f.n >= first)
        .all(|f| { f.id == 0x280 && f.protocol == Protocol::Fd && !f.brs && f.data.len() == 32 }));
}

#[test]
fn invalid_mode_switch_stops_loop_before_next_period_without_losing_data() {
    let mut c = Can {
        looping: true,
        ..Can::default()
    };
    c.cfg.protocol = Protocol::Fd;
    c.send_data = "A5 ".repeat(64);
    c.tick(Duration::from_millis(100));
    let tx = c.tx;
    assert_eq!(tx, 2);
    c.cfg.protocol = Protocol::Classic;
    c.tick(Duration::from_millis(101));
    assert!(!c.looping);
    assert_eq!(c.tx, tx);
    assert_eq!(c.send_data, "A5 ".repeat(64));
    assert_eq!(c.send_err.as_deref(), Some("经典 CAN 一帧最多 8 字节"));
    c.cfg.protocol = Protocol::Fd;
    c.tick(Duration::from_secs(1));
    assert!(!c.looping);
    assert_eq!(c.tx, tx); // Switching back never re-arms an invalidated loop.
}

struct Harness {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    texts: Vec<(String, egui::Pos2)>,
}

impl Harness {
    fn new(c: &mut Can, width: f32, height: f32, dark: bool) -> Self {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        theme::install(&ctx);
        theme::set(
            &ctx,
            if dark {
                egui::ThemePreference::Dark
            } else {
                egui::ThemePreference::Light
            },
        );
        let mut h = Self {
            ctx,
            size: egui::vec2(width, height),
            time: 0.0,
            texts: vec![],
        };
        h.frame(c, vec![]);
        h.frame(c, vec![]);
        h
    }

    fn frame(&mut self, c: &mut Can, events: Vec<egui::Event>) {
        self.time += 0.02;
        let mut out = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                focused: true,
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| {
                egui::Panel::left("nav")
                    .exact_size(crate::nav::WIDTH)
                    .show(ui, |_| {});
                egui::CentralPanel::default().show(ui, |ui| {
                    widgets::page_width(ui, |ui| page(ui, c, Duration::from_secs_f64(self.time)));
                });
            },
        );
        out.textures_delta.clear();
        self.texts.clear();
        for shape in &out.shapes {
            self.collect(&shape.shape, shape.clip_rect);
        }
        for p in self.ctx.tessellate(out.shapes, out.pixels_per_point) {
            assert!(p.clip_rect.is_finite());
            if let egui::epaint::Primitive::Mesh(m) = p.primitive {
                assert!(m.vertices.iter().all(|v| v.pos.is_finite()));
            }
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

    fn pos(&self, label: &str) -> egui::Pos2 {
        self.texts
            .iter()
            .find(|(t, _)| t == label)
            .unwrap_or_else(|| panic!("missing {label}: {:?}", self.texts))
            .1
    }

    fn click_at(&mut self, c: &mut Can, pos: egui::Pos2) {
        self.frame(c, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(
                c,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
            );
        }
        self.frame(c, vec![]);
    }

    fn click(&mut self, c: &mut Can, label: &str) {
        self.click_at(c, self.pos(label));
    }

    fn choose(&mut self, c: &mut Can, field: &str, option: &str) {
        self.click(c, field);
        let pos = self
            .texts
            .iter()
            .rev()
            .find(|(t, _)| t == option)
            .unwrap_or_else(|| panic!("missing option {option}"))
            .1;
        self.click_at(c, pos);
    }
}

#[test]
fn pointer_selects_fd_brs_rates_sends_and_restores_saved_config() {
    let mut c = Can::default();
    let mut h = Harness::new(&mut c, 1400.0, 1500.0, false);
    h.choose(&mut c, "经典 CAN", "CAN FD");
    assert_eq!(c.cfg.protocol, Protocol::Fd);
    h.choose(&mut c, "2 Mbit/s", "5 Mbit/s");
    assert_eq!(c.cfg.data_rate, 5000);
    h.click(&mut c, "数据段变速");
    assert!(!c.cfg.brs);
    h.click(&mut c, "保存配置");
    assert!(!c.saved.dirty(&c.cfg));
    h.click(&mut c, "数据段变速");
    h.choose(&mut c, "CAN FD", "经典 CAN");
    h.click(&mut c, "撤销修改");
    assert_eq!(
        (c.cfg.protocol, c.cfg.data_rate, c.cfg.brs),
        (Protocol::Fd, 5000, false)
    );
    c.send_data = "A5 ".repeat(64);
    h.frame(&mut c, vec![]);
    h.click(&mut c, "发送");
    let f = c.log.back().unwrap();
    assert_eq!(
        (f.protocol, f.brs, f.dlc, f.data.len()),
        (Protocol::Fd, false, 15, 64)
    );
    assert_eq!(c.selected, Some(f.n));
}

#[test]
fn protocol_controls_keep_positions_in_both_themes_and_supported_sizes() {
    for dark in [false, true] {
        for (w, h) in [(900.0, 600.0), (1200.0, 800.0), (1600.0, 1000.0)] {
            let mut c = Can::default();
            let mut h = Harness::new(&mut c, w, h, dark);
            let protocol = h.pos("协议模式");
            let mode = h.pos("工作模式");
            h.choose(&mut c, "经典 CAN", "CAN FD");
            assert_eq!(c.cfg.protocol, Protocol::Fd);
            assert_eq!(h.pos("协议模式"), protocol);
            assert_eq!(h.pos("工作模式"), mode);
            assert!(h.texts.iter().any(|(t, _)| t == "2 Mbit/s"));
            h.click(&mut c, "数据段变速");
            assert!(!c.cfg.brs);
            assert_eq!(h.pos("工作模式"), mode);
            h.choose(&mut c, "CAN FD", "经典 CAN");
            assert_eq!(c.cfg.protocol, Protocol::Classic);
            h.click(&mut c, "数据段变速");
            assert!(!c.cfg.brs); // Disabled in classic mode.
        }
    }
}
