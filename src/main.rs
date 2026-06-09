#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use eframe::egui;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// ── Lang ──────────────────────────────────────────────────────────────────────

struct Lang {
    // Window / panel titles
    title:           &'static str,
    // Sudo dialog
    sudo_title:      &'static str,
    sudo_placeholder:&'static str,
    btn_apply:       &'static str,
    btn_cancel:      &'static str,
    enter_password:  &'static str,
    checking:        &'static str,
    // Detail window
    detail_iface:    &'static str,  // prefix "Интерфейс: "
    loading:         &'static str,
    // Detail grid labels
    lbl_iface:       &'static str,
    lbl_type:        &'static str,
    lbl_mac:         &'static str,
    lbl_mtu:         &'static str,
    lbl_addrs:       &'static str,
    lbl_no_addrs:    &'static str,
    // Empty state
    no_ifaces:       &'static str,
}

const RU: Lang = Lang {
    title:            "Сетевые интерфейсы",
    sudo_title:       "Требуется пароль sudo",
    sudo_placeholder: "пароль sudo…",
    btn_apply:        "Применить",
    btn_cancel:       "Отмена",
    enter_password:   "Введите пароль",
    checking:         "Проверяем…",
    detail_iface:     "Интерфейс: ",
    loading:          "Загрузка…",
    lbl_iface:        "Интерфейс",
    lbl_type:         "Тип",
    lbl_mac:          "MAC",
    lbl_mtu:          "MTU",
    lbl_addrs:        "Адреса",
    lbl_no_addrs:     "нет",
    no_ifaces:        "Нет интерфейсов",
};

const EN: Lang = Lang {
    title:            "Network Interfaces",
    sudo_title:       "Sudo password required",
    sudo_placeholder: "sudo password…",
    btn_apply:        "Apply",
    btn_cancel:       "Cancel",
    enter_password:   "Enter password",
    checking:         "Checking…",
    detail_iface:     "Interface: ",
    loading:          "Loading…",
    lbl_iface:        "Interface",
    lbl_type:         "Type",
    lbl_mac:          "MAC",
    lbl_mtu:          "MTU",
    lbl_addrs:        "Addresses",
    lbl_no_addrs:     "none",
    no_ifaces:        "No interfaces",
};

fn detect_lang() -> &'static Lang {
    for var in &["LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES"] {
        if let Ok(val) = std::env::var(var) {
            if val.to_lowercase().starts_with("ru") {
                return &RU;
            }
        }
    }
    &EN
}

fn run(args: &[&str]) -> String {
    Command::new(args[0]).args(&args[1..])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string()
               + &String::from_utf8_lossy(&o.stderr).to_string())
        .unwrap_or_default()
}

fn sudo(pw: &str, args: &[&str]) -> (bool, String) {
    let mut child = match Command::new("sudo")
        .arg("-S").args(args)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn()
    { Ok(c) => c, Err(e) => return (false, e.to_string()) };
    if let Some(mut s) = child.stdin.take() {
        let _ = s.write_all(format!("{}\n", pw).as_bytes());
    }
    match child.wait_with_output() {
        Ok(o) => (o.status.success(),
                  String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr).to_string()),
        Err(e) => (false, e.to_string()),
    }
}

#[derive(Clone, Debug)]
struct Iface {
    name:     String,
    up:       bool,
    wireless: bool,
}

fn is_wireless(name: &str) -> bool {
    // Check /sys/class/net/<name>/wireless or /sys/class/net/<name>/phy80211
    std::path::Path::new(&format!("/sys/class/net/{}/wireless", name)).exists()
        || std::path::Path::new(&format!("/sys/class/net/{}/phy80211", name)).exists()
        // Also match common wireless prefixes as fallback
        || name.starts_with("wl")
        || name.starts_with("wlan")
        || name.starts_with("wifi")
        || name.starts_with("ath")
        || name.starts_with("ra")
        || name.starts_with("wlp")
        || name.starts_with("wlx")
}

fn load_ifaces() -> Vec<Iface> {
    let raw = run(&["ip", "-br", "link"]);
    raw.lines().filter_map(|line| {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 { return None; }
        let name  = parts[0].to_string();
        let state = parts[1].to_uppercase();
        let up    = state == "UP" || state == "UNKNOWN";
        let wireless = is_wireless(&name);
        Some(Iface { name, up, wireless })
    }).collect()
}

#[derive(Default, Clone)]
struct IfaceDetail {
    raw_link:  String,
    raw_addr:  String,
    raw_stats: String,
    mtu:       String,
    mac:       String,
    iface_type: String,
    rx_bytes:  String,
    tx_bytes:  String,
    addrs:     Vec<String>,
}

fn parse_detail(iface: &str) -> IfaceDetail {
    let raw_link  = run(&["ip", "-d", "link", "show", iface]);
    let raw_addr  = run(&["ip", "addr", "show", iface]);
    let raw_stats = run(&["ip", "-s", "link", "show", iface]);
    let mut d = IfaceDetail { raw_link: raw_link.clone(), raw_addr: raw_addr.clone(), raw_stats: raw_stats.clone(), ..Default::default() };

    for line in raw_link.lines() {
        let t = line.trim();
        let words: Vec<&str> = t.split_whitespace().collect();
        for (i, &w) in words.iter().enumerate() {
            if w == "mtu" { d.mtu = words.get(i+1).unwrap_or(&"").to_string(); }
        }
        if t.starts_with("link/") && words.len() >= 2 {
            d.iface_type = words[0].trim_start_matches("link/").to_string();
            d.mac = words[1].to_string();
        }
    }
    for line in raw_addr.lines() {
        let t = line.trim();
        if t.starts_with("inet ") || t.starts_with("inet6 ") {
            let words: Vec<&str> = t.split_whitespace().collect();
            if words.len() >= 2 { d.addrs.push(format!("{} {}", words[0], words[1])); }
        }
    }
    let lines: Vec<&str> = raw_stats.lines().collect();
    for (i, &line) in lines.iter().enumerate() {
        if line.trim() == "RX:" {
            if let Some(v) = lines.get(i+1) {
                d.rx_bytes = v.split_whitespace().next().unwrap_or("0").to_string();
            }
        }
        if line.trim() == "TX:" {
            if let Some(v) = lines.get(i+1) {
                d.tx_bytes = v.split_whitespace().next().unwrap_or("0").to_string();
            }
        }
    }
    d
}

fn fmt_bytes(s: &str) -> String {
    let n: u64 = s.parse().unwrap_or(0);
    if n >= 1_073_741_824 { format!("{:.1} GB", n as f64/1_073_741_824.0) }
    else if n >= 1_048_576 { format!("{:.1} MB", n as f64/1_048_576.0) }
    else if n >= 1024 { format!("{:.1} KB", n as f64/1024.0) }
    else { format!("{} B", n) }
}

// ── sudo password dialog ──────────────────────────────────────────────────────
struct SudoDialog {
    input:    String,
    error:    String,
    pending:  Option<(String, String)>, // (iface, action)
    result:   Arc<Mutex<Option<(bool, String, String)>>>, // (ok, iface, action)
}

struct DetailWindow {
    iface:  String,
    detail: Arc<Mutex<Option<IfaceDetail>>>,
}

struct App {
    ifaces:    Arc<Mutex<Vec<Iface>>>,
    loading:   Arc<Mutex<bool>>,
    last_load: Instant,
    toggling:  Arc<Mutex<std::collections::HashSet<String>>>,
    detail_win: Option<DetailWindow>,
    sudo_dlg:  Option<SudoDialog>,
    password:  Option<String>, // cached after first successful use
    lang:      &'static Lang,
}

impl App {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            ifaces:     Arc::new(Mutex::new(Vec::new())),
            loading:    Arc::new(Mutex::new(false)),
            last_load:  Instant::now() - Duration::from_secs(999),
            toggling:   Arc::new(Mutex::new(std::collections::HashSet::new())),
            detail_win: None,
            sudo_dlg:   None,
            password:   None,
            lang:       detect_lang(),
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        {
            let mut l = self.loading.lock().unwrap();
            if *l { return; }
            *l = true;
        }
        self.last_load = Instant::now();
        let ifaces  = Arc::clone(&self.ifaces);
        let loading = Arc::clone(&self.loading);
        thread::spawn(move || {
            let parsed = load_ifaces();
            *ifaces.lock().unwrap()  = parsed;
            *loading.lock().unwrap() = false;
        });
    }

    fn do_toggle(&self, pw: &str, iface: &str, action: &str) {
        let pw    = pw.to_string();
        let name  = iface.to_string();
        let act   = action.to_string();
        let tog   = Arc::clone(&self.toggling);
        let ifaces = Arc::clone(&self.ifaces);
        tog.lock().unwrap().insert(name.clone());
        thread::spawn(move || {
            sudo(&pw, &["ip","link","set",&name, &act]);
            // Poll until the interface state actually changes (max 5s)
            let target_up = act == "up";
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                thread::sleep(Duration::from_millis(200));
                let parsed = load_ifaces();
                let changed = parsed.iter()
                    .find(|i| i.name == name)
                    .map(|i| i.up == target_up)
                    .unwrap_or(true);
                *ifaces.lock().unwrap() = parsed;
                if changed || std::time::Instant::now() > deadline { break; }
            }
            tog.lock().unwrap().remove(&name);
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(500));
        let l = self.lang;

        // auto-reload
        if self.last_load.elapsed() > Duration::from_secs(10) && !*self.loading.lock().unwrap() {
            self.reload();
        }

        // ── check sudo dialog result ──────────────────────────────────────────
        if let Some(ref dlg) = self.sudo_dlg {
            let res = dlg.result.lock().unwrap().clone();
            if let Some((ok, iface, action)) = res {
                if ok {
                    // cache password and execute
                    let pw = dlg.input.clone();
                    self.password = Some(pw.clone());
                    self.do_toggle(&pw, &iface, &action);
                }
                self.sudo_dlg = None;
            }
        }



        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading(egui::RichText::new(l.title).size(17.0));
            ui.add_space(4.0);
        });

        // ── sudo dialog ───────────────────────────────────────────────────────
        if let Some(ref mut dlg) = self.sudo_dlg {
            let mut open = true;
            let iface_action = dlg.pending.clone().unwrap_or_default();

            egui::Window::new(l.sudo_title)
                .collapsible(false).resizable(false)
                .default_width(320.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "sudo ip link set {} {}",
                        iface_action.0, iface_action.1
                    ));
                    ui.add_space(8.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut dlg.input)
                            .password(true)
                            .hint_text(l.sudo_placeholder)
                            .desired_width(280.0)
                    );
                    resp.request_focus();
                    if !dlg.error.is_empty() {
                        ui.add_space(4.0);
                        ui.colored_label(egui::Color32::from_rgb(220,60,60), &dlg.error.clone());
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.button(l.btn_apply).clicked() || enter {
                            let pw    = dlg.input.clone();
                            let iface = iface_action.0.clone();
                            let act   = iface_action.1.clone();
                            let res   = Arc::clone(&dlg.result);
                            let pw2   = pw.clone();
                            thread::spawn(move || {
                                let (ok, _) = sudo(&pw2, &["true"]);
                                *res.lock().unwrap() = Some((ok, iface, act));
                            });
                            if dlg.input.is_empty() {
                                dlg.error = l.enter_password.into();
                            } else {
                                dlg.error = l.checking.into();
                            }
                        }
                        if ui.button(l.btn_cancel).clicked() {
                            *dlg.result.lock().unwrap() = Some((false, String::new(), String::new()));
                        }
                    });
                });

            if !open {
                self.sudo_dlg = None;
            }
        }

        // ── detail window ─────────────────────────────────────────────────────
        let mut close_detail = false;
        if let Some(ref dw) = self.detail_win {
            let detail_opt = dw.detail.lock().unwrap().clone();
            let mut open = true;
            egui::Window::new(format!("{}{}", l.detail_iface, dw.iface))
                .collapsible(false).resizable(true)
                .default_width(480.0).default_height(380.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    match detail_opt {
                        None => { ui.spinner(); ui.label(l.loading); }
                        Some(ref d) => {
                            egui::Grid::new("g").num_columns(2).spacing([12.0,4.0]).striped(true).show(ui, |ui| {
                                macro_rules! kv {
                                    ($k:expr, $v:expr) => {
                                        ui.label(egui::RichText::new($k).color(egui::Color32::GRAY).size(12.0));
                                        ui.label(egui::RichText::new($v).monospace().size(12.0));
                                        ui.end_row();
                                    };
                                }
                                kv!(l.lbl_iface, &dw.iface);
                                kv!(l.lbl_type,  &d.iface_type);
                                kv!(l.lbl_mac,   &d.mac);
                                kv!(l.lbl_mtu,   &d.mtu);
                                if d.addrs.is_empty() {
                                    kv!(l.lbl_addrs, l.lbl_no_addrs);
                                } else {
                                    for (i, a) in d.addrs.iter().enumerate() {
                                        kv!(if i==0 {l.lbl_addrs} else {""}, a.as_str());
                                    }
                                }
                                if !d.rx_bytes.is_empty() { kv!("RX", &fmt_bytes(&d.rx_bytes)); }
                                if !d.tx_bytes.is_empty() { kv!("TX", &fmt_bytes(&d.tx_bytes)); }
                            });
                            ui.add_space(8.0);
                            ui.collapsing("ip -d link show", |ui| {
                                egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
                                    ui.add(egui::TextEdit::multiline(&mut d.raw_link.clone())
                                        .font(egui::FontId::monospace(11.0)).desired_width(f32::INFINITY));
                                });
                            });
                            ui.collapsing("ip addr show", |ui| {
                                egui::ScrollArea::vertical().max_height(80.0).show(ui, |ui| {
                                    ui.add(egui::TextEdit::multiline(&mut d.raw_addr.clone())
                                        .font(egui::FontId::monospace(11.0)).desired_width(f32::INFINITY));
                                });
                            });
                            ui.collapsing("ip -s link show", |ui| {
                                egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
                                    ui.add(egui::TextEdit::multiline(&mut d.raw_stats.clone())
                                        .font(egui::FontId::monospace(11.0)).desired_width(f32::INFINITY));
                                });
                            });
                        }
                    }
                });
            if !open { close_detail = true; }
        }
        if close_detail { self.detail_win = None; }

        // ── main list ─────────────────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);
            let ifaces   = self.ifaces.lock().unwrap().clone();
            let toggling = self.toggling.lock().unwrap().clone();

            if ifaces.is_empty() {
                ui.centered_and_justified(|ui| {
                    if *self.loading.lock().unwrap() { ui.spinner(); }
                    else { ui.label(egui::RichText::new(l.no_ifaces).color(egui::Color32::GRAY)); }
                });
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for iface in &ifaces {
                    let is_toggling = toggling.contains(&iface.name);

                    // Allocate full-width row rect for click detection
                    let row_height = 32.0;
                    let avail_w = ui.available_width();
                    let (row_rect, row_response) = ui.allocate_exact_size(
                        egui::vec2(avail_w, row_height),
                        egui::Sense::click(),
                    );

                    // Draw contents inside the row rect
                    let mut child = ui.child_ui(row_rect, egui::Layout::left_to_right(egui::Align::Center));
                    child.add_space(8.0); // left margin

                    // LEFT: status button
                    let (label, color) = if iface.up {
                        ("UP",   egui::Color32::from_rgb(80,210,80))
                    } else {
                        ("DOWN", egui::Color32::from_rgb(210,70,70))
                    };
                    let status_btn = child.add_sized(
                        [60.0, 24.0],
                        egui::Button::new(
                            egui::RichText::new(label).color(color).strong().monospace().size(13.0)
                        ).frame(true),
                    );
                    if status_btn.clicked() && !is_toggling && self.sudo_dlg.is_none() {
                        let action = if iface.up { "down" } else { "up" };
                        if let Some(ref pw) = self.password.clone() {
                            self.do_toggle(pw, &iface.name, action);
                        } else {
                            self.sudo_dlg = Some(SudoDialog {
                                input:   String::new(),
                                error:   String::new(),
                                pending: Some((iface.name.clone(), action.to_string())),
                                result:  Arc::new(Mutex::new(None)),
                            });
                        }
                    }

                    if is_toggling { child.spinner(); }

                    // Wi-Fi button next to status, same size
                    if iface.wireless && iface.up {
                        if child.add_sized(
                            [60.0, 24.0],
                            egui::Button::new(
                                egui::RichText::new("Wi-Fi").size(13.0)
                                    .color(egui::Color32::from_rgb(100,180,255))
                                    .strong().monospace()
                            ).frame(true),
                        ).clicked() {
                            let _ = std::process::Command::new("/usr/local/bin/odjk-wifi").spawn();
                        }
                    }

                    // RIGHT: name
                    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(&iface.name).monospace().size(13.0));
                    });

                    // Full row click → detail (but not if status button was clicked)
                    if row_response.clicked() && !status_btn.clicked()
                        && self.detail_win.is_none() && self.sudo_dlg.is_none()
                    {
                        let name = iface.name.clone();
                        let det  = Arc::new(Mutex::new(None::<IfaceDetail>));
                        let det2 = Arc::clone(&det);
                        thread::spawn(move || { *det2.lock().unwrap() = Some(parse_detail(&name)); });
                        self.detail_win = Some(DetailWindow { iface: iface.name.clone(), detail: det });
                    }

                    ui.separator();
                }
            });
        });
    }
}

fn main() -> eframe::Result<()> {
    let title = detect_lang().title;
    eframe::run_native(
        title,
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([400.0, 380.0])
                .with_min_inner_size([300.0, 200.0]),
            ..Default::default()
        },
        Box::new(|cc| Box::new(App::new(cc))),
    )
}
