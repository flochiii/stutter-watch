//! Only the active catalog is decoded. Event and configuration keys stay stable.
use std::collections::BTreeMap;
// Display order: Latin names alphabetically, then Cyrillic, then East Asian scripts.
pub const LANGUAGES: [&str; 10] = ["de", "en", "es", "fr", "pl", "pt", "ru", "uk", "zh", "ja"];
pub const LANGUAGE_NAMES: [&str; 10] = [
    "Deutsch",
    "English",
    "Español",
    "Français",
    "Polski",
    "Português",
    "Русский",
    "Українська",
    "中文",
    "日本語",
];
pub struct Text {
    strings: BTreeMap<String, String>,
}
impl Text {
    pub fn new(language: &str) -> Self {
        let content = match language {
            "en" => include_str!("../locales/en.json"),
            "pl" => include_str!("../locales/pl.json"),
            "de" => include_str!("../locales/de.json"),
            "fr" => include_str!("../locales/fr.json"),
            "es" => include_str!("../locales/es.json"),
            "pt" => include_str!("../locales/pt.json"),
            "ru" => include_str!("../locales/ru.json"),
            "zh" => include_str!("../locales/zh.json"),
            "ja" => include_str!("../locales/ja.json"),
            "uk" => include_str!("../locales/uk.json"),
            _ => include_str!("../locales/en.json"),
        };
        Self {
            strings: serde_json::from_str(content).expect("validated embedded locale"),
        }
    }
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.strings.get(key).map(String::as_str).unwrap_or(key)
    }
    pub fn format(&self, key: &str, values: &[(&str, String)]) -> String {
        let mut output = self.get(key).to_owned();
        for (name, value) in values {
            output = output.replace(&format!("{{{name}}}"), value);
        }
        output
    }
    /// Translate application-generated diagnostics. API identifiers and error
    /// codes are kept as technical tokens; raw provider text stays in the log.
    pub fn diagnostic(&self, detail: &str) -> String {
        let key = if detail == "writer_queue_full"
            || detail.contains("disk queue is full")
            || detail.contains("bounded writer queue")
        {
            "queue_full"
        } else if detail.contains("free disk space") {
            "disk_reserve"
        } else if detail.contains("PDH") || detail.starts_with("pdh_error") {
            if detail.contains("initializing") {
                "pdh_initializing"
            } else if detail.contains("exceeded") {
                "pdh_slow"
            } else {
                "pdh_unavailable"
            }
        } else if detail.contains("hotkey") {
            "hotkey_unavailable"
        } else if detail.contains("tray icon") {
            "tray_unavailable"
        } else if detail.starts_with("Panel:") {
            "panel_failed"
        } else if detail.contains("ETW") || detail.contains("etw") {
            if detail.contains("WIN32_ERROR(5)") || detail.contains("0x80070005") {
                "reason_etw_permissions"
            } else {
                "reason_etw"
            }
        } else if detail.contains("NVML")
            || detail.contains("nvml")
            || detail.contains("passive_guard")
        {
            "reason_nvml"
        } else if detail.contains("GPU") {
            "reason_gpu"
        } else if detail == "active" {
            "active"
        } else if detail == "disabled" {
            "disabled"
        } else if detail == "paused" {
            "pause"
        } else {
            "operation_failed"
        };
        let codes = detail
            .split_whitespace()
            .filter(|part| part.contains("0x") || part.contains("WIN32_ERROR("))
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
        if codes.is_empty() {
            self.get(key).into()
        } else {
            format!("{} [{codes}]", self.get(key))
        }
    }
    pub fn recording_issue(&self, detail: &str) -> String {
        if detail == "writer_queue_full"
            || detail.contains("disk queue is full")
            || detail.contains("free disk space")
        {
            self.diagnostic(detail)
        } else {
            self.get("write_failed").into()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogs_have_identical_keys_and_no_empty_labels() {
        let base = Text::new("en");
        for language in LANGUAGES {
            let catalog = Text::new(language);
            assert_eq!(
                catalog.strings.keys().collect::<Vec<_>>(),
                base.strings.keys().collect::<Vec<_>>(),
                "{language}"
            );
            assert!(catalog.strings.values().all(|s| !s.trim().is_empty()));
            for (key, value) in &base.strings {
                let placeholders = |text: &str| {
                    text.split('{')
                        .skip(1)
                        .filter_map(|s| s.split_once('}').map(|(p, _)| p.to_owned()))
                        .collect::<std::collections::BTreeSet<_>>()
                };
                assert_eq!(
                    placeholders(value),
                    placeholders(catalog.get(key)),
                    "{language}: {key}"
                );
            }
        }
    }
    #[test]
    fn queue_warning_is_localized_and_preserves_the_fact_of_missing_data() {
        let en = Text::new("en").diagnostic("writer_queue_full");
        let uk = Text::new("uk").diagnostic("writer_queue_full");
        assert!(en.contains("queue is full") && en.contains("skipped"));
        assert!(uk.contains("Черга запису заповнена") && uk.contains("пропущено"));
        assert!(!uk.contains("Recording incomplete"));
        assert_eq!(Text::new("unsupported").get("apply"), "Apply");
    }
}
