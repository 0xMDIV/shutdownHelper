use chrono::{Local, NaiveTime, Datelike, Duration as ChronoDuration};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{self, BufRead, Write},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use winapi::um::winuser::{MessageBoxA, MB_OK, MB_ICONWARNING};

/// Struktur für die Konfiguration
#[derive(Serialize, Deserialize, Debug)]
struct Config {
    schedule: HashMap<String, Vec<String>>,
}

/// Läd die Konfiguration aus "config.json"
fn load_config() -> Config {
    let data = fs::read_to_string("config.json").unwrap_or_else(|_| {
        eprintln!("Konfigurationsdatei nicht gefunden. Erstelle Standardconfig.");
        r#"{
            "schedule": {
                "Monday": [],
                "Tuesday": [],
                "Wednesday": [],
                "Thursday": [],
                "Friday": [],
                "Saturday": [],
                "Sunday": []
            }
        }"#
        .to_string()
    });

    serde_json::from_str(&data).expect("Fehler beim Parsen der config.json")
}

/// Speichert die Konfiguration in "config.json"
fn save_config(config: &Config) {
    let json = serde_json::to_string_pretty(config).expect("Fehler beim Serialisieren");
    fs::write("config.json", json).expect("Fehler beim Speichern der config.json");
}

/// Zeigt eine Windows-Benachrichtigung über die MessageBox
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

/// Führt den Shutdown aus (Shutdown-Befehl: benötigt Administratorrechte)
fn shutdown_pc() {
    println!("Fahre den PC herunter...");
    let _ = Command::new("shutdown")
        .args(&["/s", "/t", "0"])
        .status();
}

/// Berechnet die Dauer (in Sekunden) von jetzt bis zur Zielzeit im Format "HH:MM"
/// Liegt die Zeit bereits in der Vergangenheit, wird morgen gerechnet.
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

/// Plant einen manuellen Shutdown (wird in einer Übersicht erfasst)
fn schedule_manual_shutdown(manual_tasks: &Arc<Mutex<Vec<(String, i64)>>>) {
    println!("Gib die gewünschte Shutdown-Zeit ein (Format HH:MM): ");
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input).unwrap();
    let input = input.trim();
    match NaiveTime::parse_from_str(input, "%H:%M") {
        Ok(target_time) => {
            let delay = get_delay_seconds(&target_time);
            println!("Shutdown geplant in {} Sekunden um {}.", delay, input);
            {
                let mut tasks = manual_tasks.lock().unwrap();
                tasks.push((input.to_string(), delay));
            }
            // Planung in einem neuen Thread
            thread::spawn(move || {
                // Falls mehr als 5 Minuten (300 sec) bis Shutdown verbleiben, zeige die Notification 5 Minuten vorher
                if delay > 300 {
                    thread::sleep(Duration::from_secs((delay - 300) as u64));
                    show_notification("Der Rechner fährt in 5 Minuten herunter! Bitte speichere deine Arbeit.");
                    thread::sleep(Duration::from_secs(300));
                } else {
                    thread::sleep(Duration::from_secs(delay as u64));
                }
                shutdown_pc();
            });
        }
        Err(_) => {
            println!("Ungültiges Format. Bitte im Format HH:MM eingeben.");
        }
    }
}

/// Plant wiederkehrende Shutdowns anhand der Konfiguration
fn activate_schedules(config: &Config) {
    // Wir starten für jeden Tag und jede Uhrzeit einen Thread
    for (day, times) in &config.schedule {
        for time_str in times {
            if let Ok(target_time) = NaiveTime::parse_from_str(time_str, "%H:%M") {
                // Finde den nächsten Wochentag, der dem Konfigurationstag entspricht
                let now = Local::now();
                let today = now.weekday().to_string(); // z. B. "Monday"
                let mut days_to_wait = 0;
                if day != &today {
                    // Wir gehen tagweise vor, bis der gewünschte Tag erreicht ist
                    let weekday_order = vec![
                        "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
                    ];
                    let today_index = weekday_order.iter().position(|&d| d == today).unwrap();
                    let target_index = weekday_order.iter().position(|&d| d == day).unwrap();
                    if target_index >= today_index {
                        days_to_wait = (target_index - today_index) as i64;
                    } else {
                        days_to_wait = (7 - today_index + target_index) as i64;
                    }
                }
                // Berechne die Verzögerung (in Sekunden) bis zur Zielzeit am geplanten Tag
                let mut delay = get_delay_seconds(&target_time) + days_to_wait * 24 * 3600;
                // Falls die berechnete Zeit schon in der Vergangenheit liegen sollte, addiere 24h
                if delay < 0 {
                    delay += 24 * 3600;
                }
                println!(
                    "Geplanter wiederkehrender Shutdown: {} um {} in {} Sekunden.",
                    day, time_str, delay
                );
                // Starte den Thread für diesen geplanten Shutdown
                thread::spawn(move || loop {
                    // Warte bis zur nächsten Ausführung:
                    if delay > 300 {
                        thread::sleep(Duration::from_secs((delay - 300) as u64));
                        show_notification("Der Rechner fährt in 5 Minuten herunter! Bitte speichere deine Arbeit.");
                        thread::sleep(Duration::from_secs(300));
                    } else {
                        thread::sleep(Duration::from_secs(delay as u64));
                    }
                    shutdown_pc();
                    // Danach plane den nächsten Shutdown in 7 Tagen (eine Woche)
                    delay = 7 * 24 * 3600;
                });
            } else {
                println!("Ungültiges Zeitformat in config für {}: {}", day, time_str);
            }
        }
    }
}

/// Hilfsfunktion, um eine Zeile von der Konsole einzulesen.
fn read_input(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input).unwrap();
    input.trim().to_string()
}

/// Bearbeitet den Zeitplan in der Konfiguration
fn edit_schedule(config: &mut Config) {
    println!("Bearbeite den Zeitplan:");
    // Zeige alle Wochentage an
    let weekdays = vec![
        "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
    ];
    for (i, day) in weekdays.iter().enumerate() {
        println!("{}: {} - {:?}", i + 1, day, config.schedule.get(*day).unwrap());
    }
    let day_choice = read_input("Welchen Tag möchtest du bearbeiten? (Zahl eingeben): ");
    if let Ok(index) = day_choice.parse::<usize>() {
        if index >= 1 && index <= weekdays.len() {
            let day = weekdays[index - 1];
            println!("Aktuelle Zeiten für {}: {:?}", day, config.schedule.get(day).unwrap());
            println!("Optionen:");
            println!("1 - Zeit hinzufügen");
            println!("2 - Alle Zeiten löschen");
            let option = read_input("Wähle eine Option (1 oder 2): ");
            match option.as_str() {
                "1" => {
                    let new_time = read_input("Gib die Shutdown-Zeit ein (Format HH:MM): ");
                    // Basis-Validierung
                    if NaiveTime::parse_from_str(&new_time, "%H:%M").is_ok() {
                        if let Some(times) = config.schedule.get_mut(day) {
                            times.push(new_time);
                        }
                    } else {
                        println!("Ungültiges Zeitformat.");
                    }
                }
                "2" => {
                    config.schedule.insert(day.to_string(), Vec::new());
                }
                _ => println!("Ungültige Option."),
            }
            save_config(config);
        } else {
            println!("Ungültige Zahl.");
        }
    } else {
        println!("Keine gültige Zahl eingegeben.");
    }
}

/// Zeigt eine Übersicht an: Wiederkehrende Shutdowns (aus config) und manuelle Shutdowns
fn show_overview(config: &Config, manual_tasks: &Arc<Mutex<Vec<(String, i64)>>>) {
    println!("\n=== Übersicht ===");
    println!("Wiederkehrende Shutdown-Zeiten (config):");
    for (day, times) in &config.schedule {
        println!("  {}: {}", day, if times.is_empty() { "Keine".to_string() } else { times.join(", ") });

    }
    println!("\nManuell geplante Shutdowns:");
    let tasks = manual_tasks.lock().unwrap();
    if tasks.is_empty() {
        println!("  Keine manuellen Shutdowns geplant.");
    } else {
        for (time_str, seconds) in tasks.iter() {
            println!("  {} (in {} Sekunden)", time_str, seconds);
        }
    }
    println!("====================\n");
}

fn main() {
    let manual_tasks: Arc<Mutex<Vec<(String, i64)>>> = Arc::new(Mutex::new(Vec::new()));
    let mut config = load_config();

    // Aktiviere die wiederkehrenden Shutdowns aus der Konfiguration
    activate_schedules(&config);

    loop {
        println!("\n--- Menü ---");
        println!("1 - Manueller Shutdown");
        println!("2 - Zeitplan anzeigen und reaktivieren");
        println!("3 - Zeitplan bearbeiten");
        println!("4 - Beenden");
        show_overview(&config, &manual_tasks);
        let choice = read_input("Wähle eine Option (1-4): ");
        match choice.as_str() {
            "1" => {
                schedule_manual_shutdown(&manual_tasks);
            }
            "2" => {
                // Läd die Config erneut und aktiviert Zeitpläne
                config = load_config();
                activate_schedules(&config);
            }
            "3" => {
                edit_schedule(&mut config);
            }
            "4" => {
                println!("Programm wird beendet.");
                std::process::exit(0);
            }
            _ => println!("Ungültige Option."),
        }
    }
}
