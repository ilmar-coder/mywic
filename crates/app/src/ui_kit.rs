//! Tema e componentes visuais do EcoReset. Os ícones são desenhados com vetores
//! porque as fontes embutidas não têm emojis nem símbolos.

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, TextStyle, Vec2, pos2, vec2};

pub const BG: Color32 = Color32::from_rgb(14, 17, 22);
pub const PANEL: Color32 = Color32::from_rgb(19, 24, 32);
pub const CARD: Color32 = Color32::from_rgb(25, 31, 42);
pub const CARD_HI: Color32 = Color32::from_rgb(35, 43, 56);
pub const BORDER: Color32 = Color32::from_rgb(42, 51, 65);
pub const TRACK: Color32 = Color32::from_rgb(46, 55, 70);
pub const TEXT: Color32 = Color32::from_rgb(230, 234, 242);
pub const MUTED: Color32 = Color32::from_rgb(140, 149, 166);
pub const ACCENT: Color32 = Color32::from_rgb(45, 212, 191);
pub const GREEN: Color32 = Color32::from_rgb(52, 211, 153);
pub const YELLOW: Color32 = Color32::from_rgb(250, 204, 21);
pub const ORANGE: Color32 = Color32::from_rgb(251, 146, 60);
pub const RED: Color32 = Color32::from_rgb(248, 113, 113);
pub const BLUE: Color32 = Color32::from_rgb(96, 165, 250);

pub fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = Color32::from_rgb(11, 14, 19);
    v.faint_bg_color = CARD_HI;
    v.code_bg_color = CARD_HI;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    for (w, fill) in [
        (&mut v.widgets.noninteractive, CARD),
        (&mut v.widgets.inactive, CARD_HI),
        (&mut v.widgets.hovered, Color32::from_rgb(44, 54, 70)),
        (&mut v.widgets.active, Color32::from_rgb(52, 63, 82)),
        (&mut v.widgets.open, Color32::from_rgb(44, 54, 70)),
    ] {
        w.corner_radius = CornerRadius::same(9);
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.text_styles.insert(TextStyle::Heading, FontId::proportional(22.0));
        s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(12.5));
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.combo_width = 160.0;
        s.spacing.scroll.floating = true;
        s.spacing.scroll.bar_width = 6.0;
    });
}

// ------------------------------------------------------------------ blocos

pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(14)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Título de página com subtítulo.
pub fn page_header(ui: &mut egui::Ui, icon: Icon, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
        ui.painter().rect_filled(r, 12, ACCENT.gamma_multiply(0.14));
        paint_icon(ui.painter(), r.shrink(10.0), icon, ACCENT);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(title).size(20.0).strong().color(TEXT));
            ui.label(RichText::new(subtitle).color(MUTED));
        });
    });
    ui.add_space(10.0);
}

pub fn section_title(ui: &mut egui::Ui, t: &str) {
    ui.label(RichText::new(t.to_uppercase()).small().strong().color(MUTED));
    ui.add_space(2.0);
}

pub fn pill(ui: &mut egui::Ui, t: &str, c: Color32) {
    egui::Frame::new()
        .fill(c.gamma_multiply(0.16))
        .corner_radius(20)
        .inner_margin(egui::Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(t).small().strong().color(c));
        });
}

/// Faixa de aviso (cor na borda esquerda).
pub fn banner(ui: &mut egui::Ui, icon: Icon, c: Color32, text: &str) {
    let r = egui::Frame::new()
        .fill(c.gamma_multiply(0.08))
        .corner_radius(10)
        .inner_margin(egui::Margin { left: 14, right: 12, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), ir, icon, c);
                ui.add(egui::Label::new(RichText::new(text).color(TEXT.gamma_multiply(0.85))).wrap());
            });
        })
        .response;
    let stripe = Rect::from_min_max(r.rect.left_top() + vec2(0.0, 8.0), pos2(r.rect.left() + 3.0, r.rect.bottom() - 8.0));
    ui.painter().rect_filled(stripe, 2, c);
}

/// Cartão de problema: faixa colorida, código, título, causa e correção.
pub fn problem_card(ui: &mut egui::Ui, code: &str, brand: &str, title: &str, cause: &str, fix: &str, service: bool) {
    let c = if service { RED } else { ORANGE };
    let r = egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(14)
        .inner_margin(egui::Margin { left: 18, right: 14, top: 12, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                pill(ui, &format!("{brand} {code}"), c);
                ui.label(RichText::new(title).strong().size(15.0));
                if service {
                    pill(ui, "assistência técnica", RED);
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Causa").small().strong().color(MUTED));
            ui.add(egui::Label::new(cause).wrap());
            ui.label(RichText::new("O que fazer").small().strong().color(MUTED));
            ui.add(egui::Label::new(fix).wrap());
        })
        .response;
    let stripe = Rect::from_min_max(r.rect.left_top() + vec2(0.0, 10.0), pos2(r.rect.left() + 4.0, r.rect.bottom() - 10.0));
    ui.painter().rect_filled(stripe, 2, c);
}

pub fn empty_state(ui: &mut egui::Ui, icon: Icon, title: &str, text: &str) {
    card(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 28.0, ACCENT.gamma_multiply(0.12));
            paint_icon(ui.painter(), r.shrink(15.0), icon, ACCENT);
            ui.add_space(6.0);
            ui.label(RichText::new(title).size(16.0).strong());
            ui.add(egui::Label::new(RichText::new(text).color(MUTED)).wrap());
            ui.add_space(10.0);
        });
    });
}

/// Logo (o mesmo desenho do ícone do app) e o nome.
pub fn brand(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, 10, Color32::from_rgb(20, 150, 140));
        let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
        arc(p, r.center(), r.width() * 0.33, 0.0, 0.82, Stroke::new(2.6, Color32::WHITE));
        paint_drop(p, Rect::from_min_max(at(0.34, 0.24), at(0.66, 0.74)), Color32::WHITE);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new("EcoReset").size(18.0).strong().color(TEXT));
            ui.label(RichText::new(format!("versão {}", ecoreset_core::VERSION)).small().color(MUTED));
        });
    });
}

// ------------------------------------------------------------------ botões

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// Botão com ícone opcional. `full` ocupa a largura toda.
pub fn button(ui: &mut egui::Ui, kind: Kind, icon: Option<Icon>, text: &str, enabled: bool, full: bool) -> egui::Response {
    let font = FontId::proportional(14.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), TEXT);
    let icon_w = if icon.is_some() { 24.0 } else { 0.0 };
    let w = if full { ui.available_width() } else { galley.size().x + icon_w + 28.0 };
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (r, resp) = ui.allocate_exact_size(vec2(w, 36.0), sense);
    let hover = enabled && resp.hovered();
    let (fill, fg, stroke) = match kind {
        Kind::Primary => (if hover { ACCENT.gamma_multiply(0.85) } else { ACCENT }, BG, Stroke::NONE),
        Kind::Danger => (if hover { RED.gamma_multiply(0.28) } else { RED.gamma_multiply(0.18) }, RED, Stroke::new(1.0, RED.gamma_multiply(0.5))),
        Kind::Secondary => (if hover { Color32::from_rgb(44, 54, 70) } else { CARD_HI }, TEXT, Stroke::new(1.0, BORDER)),
        Kind::Ghost => (if hover { CARD_HI } else { Color32::TRANSPARENT }, MUTED, Stroke::NONE),
    };
    let (fill, fg) = if enabled { (fill, fg) } else { (fill.gamma_multiply(0.45), fg.gamma_multiply(0.45)) };
    let p = ui.painter();
    p.rect(r, 10, fill, stroke, egui::StrokeKind::Inside);
    let content_w = galley.size().x + icon_w;
    let mut x = r.center().x - content_w / 2.0;
    if let Some(icon) = icon {
        paint_icon(p, Rect::from_center_size(pos2(x + 8.0, r.center().y), Vec2::splat(15.0)), icon, fg);
        x += icon_w;
    }
    p.text(pos2(x, r.center().y), egui::Align2::LEFT_CENTER, text, font, fg);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

pub fn primary(ui: &mut egui::Ui, icon: Icon, text: &str, enabled: bool) -> egui::Response {
    button(ui, Kind::Primary, Some(icon), text, enabled, false)
}

pub fn secondary(ui: &mut egui::Ui, icon: Icon, text: &str, enabled: bool) -> egui::Response {
    button(ui, Kind::Secondary, Some(icon), text, enabled, false)
}

pub fn danger(ui: &mut egui::Ui, icon: Icon, text: &str, enabled: bool) -> egui::Response {
    button(ui, Kind::Danger, Some(icon), text, enabled, false)
}

/// Controle de duas ou mais opções lado a lado. Devolve true se mudou.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, Icon, &str)], full: bool) -> bool {
    let n = options.len() as f32;
    let w = if full { ui.available_width() } else { options.iter().map(|o| o.2.len() as f32 * 7.6 + 46.0).sum() };
    let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
    ui.painter().rect_filled(r, 10, Color32::from_rgb(11, 14, 19));
    let mut changed = false;
    let seg_w = (w - 6.0) / n;
    for (i, (opt, icon, label)) in options.iter().enumerate() {
        let sr = Rect::from_min_size(pos2(r.left() + 3.0 + seg_w * i as f32, r.top() + 3.0), vec2(seg_w, 28.0));
        let resp = ui.interact(sr, ui.scope_id().with(("seg", label)), Sense::click());
        let selected = *value == *opt;
        let (fill, fg) = if selected { (CARD_HI, TEXT) } else if resp.hovered() { (CARD, TEXT) } else { (Color32::TRANSPARENT, MUTED) };
        ui.painter().rect_filled(sr, 8, fill);
        let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(13.5), fg);
        let cw = galley.size().x + 20.0;
        let x = sr.center().x - cw / 2.0;
        paint_icon(ui.painter(), Rect::from_center_size(pos2(x + 7.0, sr.center().y), Vec2::splat(13.0)), *icon, if selected { ACCENT } else { fg });
        ui.painter().galley(pos2(x + 20.0, sr.center().y - galley.size().y / 2.0), galley, fg);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, *label));
        if resp.clicked() && !selected {
            *value = *opt;
            changed = true;
        }
    }
    changed
}

/// Item da navegação lateral.
pub fn nav_item(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let (fill, fg) = if selected {
        (ACCENT.gamma_multiply(0.13), ACCENT)
    } else if resp.hovered() {
        (CARD, TEXT)
    } else {
        (Color32::TRANSPARENT, MUTED)
    };
    ui.painter().rect_filled(r, 10, fill);
    if selected {
        ui.painter().rect_filled(Rect::from_min_size(pos2(r.left(), r.top() + 9.0), vec2(3.0, 20.0)), 2, ACCENT);
    }
    paint_icon(ui.painter(), Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), Vec2::splat(16.0)), icon, fg);
    ui.painter().text(pos2(r.left() + 42.0, r.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(14.0), if selected { TEXT } else { fg });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Campo de texto com rótulo em cima.
pub fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, width: f32) -> egui::Response {
    let mut out = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        let l = ui.label(RichText::new(label).small().color(MUTED));
        let r = ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(width).margin(vec2(8.0, 6.0)));
        out = Some(r.labelled_by(l.id));
    });
    out.unwrap()
}

/// Medidor circular de porcentagem (almofada).
pub fn gauge(ui: &mut egui::Ui, label: &str, pct: f64) {
    let c = if pct >= 100.0 {
        RED
    } else if pct >= 80.0 {
        ORANGE
    } else if pct >= 50.0 {
        YELLOW
    } else {
        GREEN
    };
    let (r, _) = ui.allocate_exact_size(vec2(150.0, 170.0), Sense::hover());
    let center = pos2(r.center().x, r.top() + 72.0);
    let p = ui.painter();
    arc(p, center, 58.0, -0.375, 0.375, Stroke::new(10.0, TRACK));
    let frac = (pct / 100.0).clamp(0.0, 1.0) as f32;
    if frac > 0.0 {
        arc(p, center, 58.0, -0.375, -0.375 + 0.75 * frac, Stroke::new(10.0, c));
    }
    p.text(center, egui::Align2::CENTER_CENTER, format!("{pct:.1}%"), FontId::proportional(24.0), TEXT);
    p.text(pos2(center.x, center.y + 26.0), egui::Align2::CENTER_CENTER, "cheia", FontId::proportional(11.5), MUTED);
    p.text(pos2(center.x, r.bottom() - 18.0), egui::Align2::CENTER_CENTER, label, FontId::proportional(13.5), TEXT);
}

// ------------------------------------------------------------------ ícones

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Pulse,
    Drop,
    Wrench,
    Chip,
    Database,
    Usb,
    Wifi,
    Printer,
    Search,
    Refresh,
    Trash,
    Folder,
    File,
    Key,
    Check,
    Alert,
    Nozzle,
    Sparkle,
    Upload,
    Download,
    Compare,
    Cloud,
    Reset,
    Undo,
    Stop,
    Save,
    Pen,
    Info,
}

/// Arco de `t0` a `t1` (frações de volta, 0 = topo, sentido horário).
pub fn arc(p: &egui::Painter, c: Pos2, r: f32, t0: f32, t1: f32, stroke: Stroke) {
    let n = ((t1 - t0).abs() * 96.0).max(2.0) as usize;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f32 / n as f32;
            let a = -std::f32::consts::FRAC_PI_2 + t * std::f32::consts::TAU;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(Shape::line(pts, stroke));
}

fn paint_drop(p: &egui::Painter, r: Rect, c: Color32) {
    let w = r.width();
    let bottom = pos2(r.center().x, r.bottom() - w * 0.5);
    p.circle_filled(bottom, w * 0.5, c);
    p.add(Shape::convex_polygon(
        vec![pos2(r.center().x, r.top()), pos2(r.right() - w * 0.035, bottom.y - w * 0.12), pos2(r.left() + w * 0.035, bottom.y - w * 0.12)],
        c,
        Stroke::NONE,
    ));
}

fn arrow_head(p: &egui::Painter, tip: Pos2, dir: Vec2, size: f32, c: Color32) {
    let d = dir.normalized();
    let n = vec2(-d.y, d.x);
    p.add(Shape::convex_polygon(vec![tip, tip - d * size + n * size * 0.6, tip - d * size - n * size * 0.6], c, Stroke::NONE));
}

pub fn paint_icon(p: &egui::Painter, r: Rect, icon: Icon, c: Color32) {
    let w = r.width();
    let s = Stroke::new((w * 0.1).max(1.5), c);
    let thin = Stroke::new((w * 0.075).max(1.2), c);
    let at = |x: f32, y: f32| pos2(r.left() + x * w, r.top() + y * r.height());
    match icon {
        Icon::Pulse => {
            let pts = vec![at(0.0, 0.55), at(0.25, 0.55), at(0.38, 0.2), at(0.55, 0.9), at(0.68, 0.45), at(0.78, 0.55), at(1.0, 0.55)];
            p.add(Shape::line(pts, s));
        }
        Icon::Drop => {
            paint_drop(p, Rect::from_min_max(at(0.18, 0.0), at(0.82, 1.0)), c);
        }
        Icon::Wrench => {
            // cabo na diagonal e a boca aberta para cima à direita
            p.line_segment([at(0.12, 0.88), at(0.56, 0.44)], Stroke::new(w * 0.17, c));
            arc(p, at(0.7, 0.3), w * 0.2, 0.25, 0.95, Stroke::new(w * 0.14, c));
        }
        Icon::Chip => {
            let rr = Rect::from_min_max(at(0.2, 0.2), at(0.8, 0.8));
            p.rect_stroke(rr, 2, s, egui::StrokeKind::Middle);
            p.rect_filled(rr.shrink(w * 0.17), 1, c);
            for t in [0.35, 0.5, 0.65] {
                p.line_segment([at(t, 0.0), at(t, 0.2)], thin);
                p.line_segment([at(t, 0.8), at(t, 1.0)], thin);
                p.line_segment([at(0.0, t), at(0.2, t)], thin);
                p.line_segment([at(0.8, t), at(1.0, t)], thin);
            }
        }
        Icon::Database => {
            for y in [0.15, 0.5, 0.85] {
                p.add(Shape::ellipse_stroke(at(0.5, y), vec2(w * 0.4, w * 0.14), s));
            }
            p.line_segment([at(0.1, 0.15), at(0.1, 0.85)], s);
            p.line_segment([at(0.9, 0.15), at(0.9, 0.85)], s);
        }
        Icon::Usb => {
            p.line_segment([at(0.5, 0.12), at(0.5, 0.84)], s);
            arrow_head(p, at(0.5, 0.0), vec2(0.0, -1.0), w * 0.2, c);
            p.line_segment([at(0.5, 0.62), at(0.22, 0.44)], s);
            p.line_segment([at(0.22, 0.44), at(0.22, 0.32)], s);
            p.circle_filled(at(0.22, 0.27), w * 0.08, c);
            p.line_segment([at(0.5, 0.52), at(0.78, 0.36)], s);
            p.rect_filled(Rect::from_center_size(at(0.78, 0.3), Vec2::splat(w * 0.15)), 1, c);
            p.circle_filled(at(0.5, 0.9), w * 0.1, c);
        }
        Icon::Wifi => {
            let ctr = at(0.5, 0.88);
            for rad in [0.3, 0.55, 0.8] {
                arc(p, ctr, w * rad, -0.125, 0.125, s);
            }
            p.circle_filled(ctr, w * 0.08, c);
        }
        Icon::Printer => {
            p.rect_stroke(Rect::from_min_max(at(0.25, 0.0), at(0.75, 0.3)), 1, thin, egui::StrokeKind::Middle);
            p.rect_stroke(Rect::from_min_max(at(0.0, 0.3), at(1.0, 0.72)), 3, s, egui::StrokeKind::Middle);
            p.rect_filled(Rect::from_min_max(at(0.25, 0.58), at(0.75, 1.0)), 1, c);
            p.circle_filled(at(0.82, 0.44), w * 0.05, c);
        }
        Icon::Search => {
            p.circle_stroke(at(0.42, 0.42), w * 0.3, s);
            p.line_segment([at(0.64, 0.64), at(0.95, 0.95)], Stroke::new(w * 0.14, c));
        }
        Icon::Refresh | Icon::Reset => {
            let ctr = r.center();
            arc(p, ctr, w * 0.38, 0.05, 0.8, s);
            let a = -std::f32::consts::FRAC_PI_2 + 0.8 * std::f32::consts::TAU;
            let tip = ctr + vec2(a.cos(), a.sin()) * w * 0.38;
            arrow_head(p, tip + vec2(0.0, -w * 0.06), vec2(a.sin(), -a.cos()) * -1.0 + vec2(0.4, -0.6), w * 0.26, c);
            if icon == Icon::Reset {
                p.circle_filled(ctr, w * 0.1, c);
            }
        }
        Icon::Undo => {
            arc(p, at(0.55, 0.55), w * 0.33, 0.75, 1.25, s);
            p.line_segment([at(0.55, 0.88), at(0.3, 0.88)], s);
            arrow_head(p, at(0.12, 0.22), vec2(-1.0, 0.0), w * 0.24, c);
            p.line_segment([at(0.22, 0.22), at(0.55, 0.22)], s);
        }
        Icon::Trash => {
            p.line_segment([at(0.08, 0.2), at(0.92, 0.2)], s);
            p.line_segment([at(0.38, 0.05), at(0.62, 0.05)], s);
            p.add(Shape::convex_polygon(vec![at(0.18, 0.28), at(0.82, 0.28), at(0.74, 1.0), at(0.26, 1.0)], Color32::TRANSPARENT, s));
        }
        Icon::Folder => {
            p.add(Shape::convex_polygon(vec![at(0.0, 0.15), at(0.38, 0.15), at(0.48, 0.28), at(1.0, 0.28), at(1.0, 0.88), at(0.0, 0.88)], Color32::TRANSPARENT, s));
        }
        Icon::File => {
            p.add(Shape::convex_polygon(vec![at(0.15, 0.0), at(0.62, 0.0), at(0.85, 0.24), at(0.85, 1.0), at(0.15, 1.0)], Color32::TRANSPARENT, s));
            for y in [0.45, 0.62, 0.79] {
                p.line_segment([at(0.3, y), at(0.7, y)], thin);
            }
        }
        Icon::Key => {
            p.circle_stroke(at(0.28, 0.5), w * 0.22, s);
            p.line_segment([at(0.5, 0.5), at(1.0, 0.5)], s);
            p.line_segment([at(0.82, 0.5), at(0.82, 0.72)], s);
            p.line_segment([at(0.96, 0.5), at(0.96, 0.68)], s);
        }
        Icon::Check => {
            p.add(Shape::line(vec![at(0.08, 0.52), at(0.38, 0.82), at(0.94, 0.2)], Stroke::new(w * 0.14, c)));
        }
        Icon::Alert => {
            p.add(Shape::convex_polygon(vec![at(0.5, 0.02), at(1.0, 0.95), at(0.0, 0.95)], Color32::TRANSPARENT, s));
            p.line_segment([at(0.5, 0.35), at(0.5, 0.65)], s);
            p.circle_filled(at(0.5, 0.8), w * 0.065, c);
        }
        Icon::Info => {
            p.circle_stroke(r.center(), w * 0.46, s);
            p.line_segment([at(0.5, 0.45), at(0.5, 0.75)], s);
            p.circle_filled(at(0.5, 0.28), w * 0.065, c);
        }
        Icon::Nozzle => {
            for (i, x) in [0.12, 0.37, 0.62, 0.87].iter().enumerate() {
                for (j, y) in [0.15, 0.5, 0.85].iter().enumerate() {
                    if !(i == 2 && j == 1) {
                        p.line_segment([at(*x, *y - 0.1), at(*x, *y + 0.1)], s);
                    }
                }
            }
        }
        Icon::Sparkle => {
            let star = |cx: f32, cy: f32, k: f32| {
                vec![at(cx, cy - k), at(cx + k * 0.28, cy - k * 0.28), at(cx + k, cy), at(cx + k * 0.28, cy + k * 0.28), at(cx, cy + k), at(cx - k * 0.28, cy + k * 0.28), at(cx - k, cy), at(cx - k * 0.28, cy - k * 0.28)]
            };
            p.add(Shape::convex_polygon(star(0.42, 0.55, 0.42), c, Stroke::NONE));
            p.add(Shape::convex_polygon(star(0.82, 0.18, 0.18), c, Stroke::NONE));
        }
        Icon::Upload | Icon::Download => {
            p.line_segment([at(0.08, 0.72), at(0.08, 0.95)], s);
            p.line_segment([at(0.08, 0.95), at(0.92, 0.95)], s);
            p.line_segment([at(0.92, 0.95), at(0.92, 0.72)], s);
            p.line_segment([at(0.5, 0.05), at(0.5, 0.7)], s);
            if icon == Icon::Upload {
                arrow_head(p, at(0.5, 0.0), vec2(0.0, -1.0), w * 0.28, c);
            } else {
                arrow_head(p, at(0.5, 0.78), vec2(0.0, 1.0), w * 0.28, c);
            }
        }
        Icon::Compare => {
            p.line_segment([at(0.05, 0.3), at(0.85, 0.3)], s);
            arrow_head(p, at(1.0, 0.3), vec2(1.0, 0.0), w * 0.24, c);
            p.line_segment([at(0.15, 0.72), at(0.95, 0.72)], s);
            arrow_head(p, at(0.0, 0.72), vec2(-1.0, 0.0), w * 0.24, c);
        }
        Icon::Cloud => {
            p.circle_filled(at(0.32, 0.6), w * 0.22, c);
            p.circle_filled(at(0.56, 0.45), w * 0.28, c);
            p.circle_filled(at(0.78, 0.62), w * 0.2, c);
            p.rect_filled(Rect::from_min_max(at(0.32, 0.6), at(0.78, 0.82)), 0, c);
        }
        Icon::Stop => {
            p.rect_filled(r.shrink(w * 0.12), 3, c);
        }
        Icon::Save => {
            p.rect_stroke(r.shrink(w * 0.06), 3, s, egui::StrokeKind::Middle);
            p.rect_filled(Rect::from_min_max(at(0.28, 0.06), at(0.72, 0.36)), 1, c);
            p.rect_stroke(Rect::from_min_max(at(0.24, 0.58), at(0.76, 0.94)), 1, thin, egui::StrokeKind::Middle);
        }
        Icon::Pen => {
            p.line_segment([at(0.2, 0.8), at(0.85, 0.15)], Stroke::new(w * 0.18, c));
            p.add(Shape::convex_polygon(vec![at(0.05, 0.95), at(0.12, 0.72), at(0.28, 0.88)], c, Stroke::NONE));
        }
    }
}
