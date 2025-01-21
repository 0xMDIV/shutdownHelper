#![allow(deprecated)]

use chrono::{Local, NaiveTime, Datelike, Duration as ChronoDuration};
use eframe::{egui, App};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use winapi::um::winuser::{MessageBoxA, MB_OK, MB_ICONWARNING};
use regex::Regex;


/// Konfigurationsstruktur: enthält den Zeitplan für jeden Wochentag
#[derive(Serialize, Deserialize, Debug, Default)]
struct Config {
    // Für jeden Wochentag wird ein Array gespeichert. Wir nutzen hier nur das erste Element.
    schedule: HashMap<String, Vec<String>>,
}

/// Hilfsfunktion: Läd die Config aus config.json oder erzeugt einen Default.
fn load_config() -> Config {
    fs::read_to_string("config.json")
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_else(|| {
            let mut schedule = HashMap::new();
            for day in &[
                "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
            ] {
                schedule.insert(day.to_string(), vec!["".to_string()]);
            }
            Config { schedule }
        })
}

/// Hilfsfunktion: Speichert die Config in config.json.
fn save_config(config: &Config) {
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = fs::write("config.json", json);
    }
}

/// Zeigt eine Windows-Benachrichtigung via MessageBox (winapi).
fn show_notification(message: &str) {
    use std::ffi::CString;
    let c_message = CString::new(message).unwrap();
    let c_title = CString::new("Shutdown Warning").unwrap();
    unsafe {
        MessageBoxA(
            std::ptr::null_mut(),
            c_message.as_ptr(),
            c_title.as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}

/// Führt den Shutdown aus (benötigt Administratorrechte).
fn shutdown_pc() {
    println!("Fahre den PC herunter...");
    let _ = Command::new("shutdown").args(&["/s", "/t", "0"]).status();
}

/// Berechnet die Anzahl der Sekunden von jetzt bis zur Zielzeit (Format "HH:MM").
/// Ist die Zielzeit bereits vergangen, wird der morgige Tag angenommen.
fn get_delay_seconds(target_time: &NaiveTime) -> i64 {
    let now = Local::now();
    let mut target = now.date().and_time(*target_time);
    if let Some(t) = target {
        if t <= now {
            target = Some(t + ChronoDuration::days(1));
        }
    }
    (target.unwrap() - now).num_seconds()
}

/// Struktur für die GUI. Zusätzlich zur bisherigen manuellen Shutdown-Funktionalität
/// enthält diese Version ein Bearbeitungsformular für den Zeitplan.
struct ShutdownApp {
    // Für manuellen Shutdown
    manual_input: String,
    manual_status: String,
    // Übersicht manuell geplanter Shutdowns (für dieses Beispiel nicht weiter genutzt)
    manual_tasks: Arc<Mutex<Vec<(String, i64)>>>,
    // Config (Zeitplan)
    config: Config,
    // Status-Nachricht zum Zeitplan
    schedule_status: String,
    // Für jeden Wochentag: (aktiv, shutdown_time)
    schedule_edit: HashMap<String, (bool, String)>,
}

impl Default for ShutdownApp {
    fn default() -> Self {
        let config = load_config();
        let mut schedule_edit = HashMap::new();
        // Erstelle Bearbeitungsdaten basierend auf config
        for day in &[
            "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
        ] {
            // Wir erwarten ein einzelnes Element im Array
            let time_opt = config.schedule.get(*day).and_then(|v| v.get(0));
            let active = match time_opt {
                Some(t) if !t.trim().is_empty() => true,
                _ => false,
            };
            let time_value = time_opt.cloned().unwrap_or_else(|| "".to_string());
            schedule_edit.insert(day.to_string(), (active, time_value));
        }
        Self {
            manual_input: "".to_owned(),
            manual_status: "Kein manueller Shutdown geplant.".to_owned(),
            manual_tasks: Arc::new(Mutex::new(Vec::new())),
            config,
            schedule_status: "".to_owned(),
            schedule_edit,
        }
    }
}

impl ShutdownApp {
    /// Plant einen manuellen Shutdown anhand der eingegebenen Zeit.
    fn schedule_manual_shutdown(&mut self) {
        if let Ok(target_time) = NaiveTime::parse_from_str(self.manual_input.trim(), "%H:%M") {
            let delay = get_delay_seconds(&target_time);
            self.manual_status = format!("Shutdown in {} Sekunden geplant um {}.", delay, self.manual_input);
            {
                let mut tasks = self.manual_tasks.lock().unwrap();
                tasks.push((self.manual_input.clone(), delay));
            }
            thread::spawn(move || {
                if delay > 300 {
                    thread::sleep(Duration::from_secs((delay - 300) as u64));
                    show_notification("Der Rechner fährt in 5 Minuten herunter!\nBitte speichere deine Arbeit.");
                    thread::sleep(Duration::from_secs(300));
                } else {
                    thread::sleep(Duration::from_secs(delay as u64));
                }
                shutdown_pc();
            });
        } else {
            self.manual_status = "Ungültiges Zeitformat! Bitte HH:MM eingeben.".to_owned();
        }
    }

    /// Aktiviert wiederkehrende Shutdowns anhand der Konfiguration.
    /// In diesem Beispiel wird jeweils nur eine Zeit pro Tag verwendet.
    fn activate_schedules(&mut self) {
        let schedule = self.config.schedule.clone();
        for (day, times) in schedule {
            // Wir nutzen hier nur den ersten Eintrag
            if let Some(time_str) = times.get(0) {
                if !time_str.trim().is_empty() {
                    if let Ok(target_time) = NaiveTime::parse_from_str(time_str, "%H:%M") {
                        // Berechne, wie viele Tage bis zum gewünschten Wochentag gewartet werden müssen.
                        let now = Local::now();
                        let today = now.weekday().to_string(); // z. B. "Monday"
                        let weekday_order = vec![
                            "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
                        ];
                        let today_index = match weekday_order.iter().position(|&d| d == today) {
                            Some(idx) => idx,
                            None => {
                                eprintln!("Fehler: Heute ('{}') ist nicht in weekday_order enthalten!", today);
                                continue;
                            }
                        };
                        
                        // Position des Zieltages ermitteln
                        let target_index = match weekday_order.iter().position(|&d| d == day) {
                            Some(idx) => idx,
                            None => {
                                eprintln!("Fehler: Zieltag '{}' nicht in weekday_order enthalten!", day);
                                continue;
                            }
                        };
                        let days_to_wait = if target_index >= today_index {
                            target_index - today_index
                        } else {
                            7 - today_index + target_index
                        } as i64;
                        let mut delay = get_delay_seconds(&target_time) + days_to_wait * 24 * 3600;
                        if delay < 0 {
                            delay += 24 * 3600;
                        }
                        println!("Geplanter Shutdown: {} um {} in {} Sekunden.", day, time_str, delay);
                        thread::spawn(move || loop {
                            if delay > 300 {
                                thread::sleep(Duration::from_secs((delay - 300) as u64));
                                show_notification("Der Rechner fährt in 5 Minuten herunter!\nBitte speichere deine Arbeit.");
                                thread::sleep(Duration::from_secs(300));
                            } else {
                                thread::sleep(Duration::from_secs(delay as u64));
                            }
                            shutdown_pc();
                            // Nächster Shutdown in 7 Tagen
                            delay = 7 * 24 * 3600;
                        });
                    } else {
                        println!("Ungültiges Zeitformat in config für {}: {}", day, time_str);
                    }
                }
            }
        }
        self.schedule_status = "Wiederkehrende Shutdowns aktiviert.".to_owned();
    }

    /// Speichert die Zeitplan-Bearbeitungsdaten in die Config
    /// und aktualisiert den wiederkehrenden Zeitplan.
    fn save_schedule(&mut self) {
        // Erstelle einen Regex, der das Format HH:MM validiert.
        // ^\d{2}:\d{2}$ bedeutet: genau 2 Ziffern, ein Doppelpunkt, genau 2 Ziffern.
        let time_regex = Regex::new(r"^\d{2}:\d{2}$").unwrap();

        for (day, (active, time)) in &self.schedule_edit {
            if *active {
                if time_regex.is_match(time.trim()) {
                    self.config.schedule.insert(day.clone(), vec![time.trim().to_string()]);
                } else {
                    // Falls das Format ungültig ist, kannst du auch standardmäßig einen leeren Wert
                    // einsetzen oder eine Fehlermeldung setzen.
                    println!("Ungültiges Zeitformat für {}: {}. Bitte gib HH:MM ein.", day, time);
                    self.config.schedule.insert(day.clone(), vec!["".to_string()]);
                }
            } else {
                self.config.schedule.insert(day.clone(), vec!["".to_string()]);
            }
        }
        save_config(&self.config);
        self.activate_schedules();
        self.schedule_status = "Zeitplan gespeichert und aktiviert.".to_string();
    }
}

impl App for ShutdownApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Shutdown GUI App");
            ui.separator();

            // Manueller Shutdown
            ui.label("Manueller Shutdown (Format HH:MM):");
            ui.text_edit_singleline(&mut self.manual_input);
            if ui.button("Shutdown manuell planen").clicked() {
                self.schedule_manual_shutdown();
            }
            ui.label(&self.manual_status);

            ui.separator();
            // Übersicht des Wiederkehrenden Zeitplans (aus config.json)
            ui.heading("Wiederkehrender Zeitplan");
            for (day, times) in &self.config.schedule {
                ui.horizontal(|ui| {
                    ui.label(format!("{}:", day));
                    if times.is_empty() || times[0].trim().is_empty() {
                        ui.label("Nicht aktiviert".to_string());
                    } else {
                        ui.label(times.join(", "));
                    }
                });
            }
            if ui.button("Zeitplan neu laden und aktivieren").clicked() {
                self.config = load_config();
                self.activate_schedules();
            }
            ui.label(&self.schedule_status);

            ui.separator();
            // Bearbeiten des Zeitplans – für jeden Wochentag
            ui.heading("Zeitplan bearbeiten");
            for day in &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"] {
                if let Some((active, time)) = self.schedule_edit.get_mut(&day.to_string()) {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", day));
                        ui.checkbox(active, "");
                        ui.label("Uhrzeit (HH:MM):");
                        ui.text_edit_singleline(time);
                    });
                }
            }
            if ui.button("Zeitplan speichern").clicked() {
                self.save_schedule();
            }
        });
    }
}

fn main() {
    let app = ShutdownApp::default();
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Shutdown GUI App",
        native_options,
        Box::new(|_cc| Box::new(app)),
    );
}
