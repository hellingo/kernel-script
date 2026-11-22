//! UI language: Turkish (default) or English.
//!
//! Both tables are `Strings` structs, so a key that exists in one language
//! must exist in the other — a missing translation is a compile error, not
//! a blank label. The choice is process-wide (one atomic: the UI renders
//! on a single thread and the worker only reads it), toggled from the
//! top-right corner of `ks-inspector`'s tab row (and of the overlay's
//! control window), and persisted in the Lua config store as
//! `gui.language` (`"tr"` / `"en"`). Both apps read the same `config.json`
//! when they live in one directory, so the choice carries over.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::config_store::{ConfigValue, SharedConfigStore};

/// Config-store key holding the selected language code.
pub const CONFIG_KEY: &str = "gui.language";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lang {
    Tr,
    En,
}

const TR_TAG: u8 = 0;
const EN_TAG: u8 = 1;

static CURRENT: AtomicU8 = AtomicU8::new(TR_TAG);

/// The active language.
pub fn lang() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == EN_TAG {
        Lang::En
    } else {
        Lang::Tr
    }
}

/// The string table for the active language.
pub fn s() -> &'static Strings {
    match lang() {
        Lang::Tr => &TR,
        Lang::En => &EN,
    }
}

/// The persisted code for `lang` (`"tr"` / `"en"`).
pub fn code(lang: Lang) -> &'static str {
    match lang {
        Lang::Tr => "tr",
        Lang::En => "en",
    }
}

/// Switches the language without touching the config store.
pub fn set_lang(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::Tr => TR_TAG,
            Lang::En => EN_TAG,
        },
        Ordering::Relaxed,
    );
}

/// Switches the language and persists the choice; a store that cannot be
/// locked still gets the in-memory switch.
pub fn set_lang_persist(lang: Lang, config: &SharedConfigStore) {
    set_lang(lang);
    if let Ok(mut store) = config.lock() {
        store.set(CONFIG_KEY, ConfigValue::Str(code(lang).to_owned()));
        store.flush();
    }
}

/// Restores the persisted language, if one is recorded. Unknown values
/// leave the default (Turkish) in place.
pub fn restore(config: &SharedConfigStore) {
    let Ok(store) = config.lock() else {
        return;
    };
    if let Some(ConfigValue::Str(code)) = store.get(CONFIG_KEY) {
        match code.as_str() {
            "en" => set_lang(Lang::En),
            "tr" | "zh" => set_lang(Lang::Tr), // Accept saved Chinese settings.
            _ => {}
        }
    }
}

/// Fills the `{}` placeholders of a table string, in order. `format!`
/// requires a literal format string, and the two languages may order the
/// parts differently, so the holes are positional.
pub fn fill(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(template.len() + 16);
    let mut parts = template.split("{}");
    if let Some(head) = parts.next() {
        out.push_str(head);
    }
    for (arg, tail) in args.iter().zip(parts) {
        let _ = write!(out, "{arg}");
        out.push_str(tail);
    }
    out
}

/// Draws the Turkish / English toggle.
/// `ks-inspector` right-aligns it in its tab row; the overlay's compact
/// control window keeps it in a plain horizontal row.
pub fn toggle_widget(ui: &mut egui::Ui, config: &SharedConfigStore) {
    let current = lang();
    if ui.selectable_label(current == Lang::En, "EN").clicked() && current != Lang::En {
        set_lang_persist(Lang::En, config);
    }
    if ui.selectable_label(current == Lang::Tr, "TR").clicked() && current != Lang::Tr {
        set_lang_persist(Lang::Tr, config);
    }
}

/// Every UI string in one language.
pub struct Strings {
    // Window chrome and the overlay control window.
    pub app_title: &'static str,
    pub overlay_title: &'static str,
    pub show_fps: &'static str,
    pub ui_scale: &'static str,
    pub reset_ui_scale: &'static str,
    pub quit: &'static str,
    // Inspector chrome.
    pub log: &'static str,
    pub process: &'static str,
    pub pid: &'static str,
    pub filter_hint: &'static str,
    pub refresh: &'static str,
    pub close: &'static str,
    pub tab_hex: &'static str,
    pub tab_editor: &'static str,
    pub tab_modify: &'static str,
    // Hex tab.
    pub hex_start: &'static str,
    pub hex_no_base: &'static str,
    pub length: &'static str,
    /// `{}` is the byte count.
    pub bytes_fmt: &'static str,
    pub prev_page: &'static str,
    pub next_page: &'static str,
    /// `{}` is the 1-based page number.
    pub page_fmt: &'static str,
    pub reset_base: &'static str,
    pub hex_pick_process: &'static str,
    pub reading: &'static str,
    /// `{}` is the driver error.
    pub read_failed_fmt: &'static str,
    // Editor tab.
    pub script_dir: &'static str,
    pub reload: &'static str,
    pub reloaded: &'static str,
    pub new_file_hint: &'static str,
    pub new_file: &'static str,
    pub save: &'static str,
    pub unsaved: &'static str,
    /// `{}` = script count, second `{}` = directory.
    pub scanned_fmt: &'static str,
    /// `{}` is the saved file name.
    pub saved_fmt: &'static str,
    /// `{}` is the OS error.
    pub save_failed_fmt: &'static str,
    pub invalid_name: &'static str,
    pub file_exists: &'static str,
    /// `{}` is the new file name.
    pub created_fmt: &'static str,
    /// `{}` is the OS error.
    pub create_failed_fmt: &'static str,
    // Modify tab.
    /// `{}` is the lock-count suffix (or empty).
    pub lock_table_fmt: &'static str,
    pub unlock_all: &'static str,
    pub lock_hint: &'static str,
    pub no_locks: &'static str,
    pub lock_id: &'static str,
    pub address: &'static str,
    pub source: &'static str,
    pub this_window: &'static str,
    pub lua_other: &'static str,
    pub unlock: &'static str,
    pub add: &'static str,
    pub refresh_values: &'static str,
    pub write_changes: &'static str,
    pub unfreeze_all: &'static str,
    pub col_freeze: &'static str,
    pub col_type: &'static str,
    pub col_value: &'static str,
    pub no_process: &'static str,
    pub delete: &'static str,
    // Value types.
    pub type_i8: &'static str,
    pub type_i16: &'static str,
    pub type_i32: &'static str,
    pub type_i64: &'static str,
    pub type_u32: &'static str,
    pub type_f32: &'static str,
    pub type_f64: &'static str,
    // Transient statuses (set when an event arrives).
    pub status_initial: &'static str,
    pub written: &'static str,
    pub freezing: &'static str,
    pub unfrozen: &'static str,
    pub all_unfrozen: &'static str,
    pub unlocked: &'static str,
    /// `{}` is the process count.
    pub process_list_fmt: &'static str,
    /// `{}` is the driver error.
    pub base_failed_fmt: &'static str,
    /// `{}` is the lock count.
    pub entries_fmt: &'static str,
}

static TR: Strings = Strings {
    app_title: "KernelScript Inspector - Bellek Aracı",
    overlay_title: "KernelScript",
    show_fps: "FPS göster",
    ui_scale: "Arayüz ölçeği",
    reset_ui_scale: "Varsayılana dön",
    quit: "Çıkış",
    log: "Günlük",
    process: "Süreç",
    pid: "PID",
    filter_hint: "Filtrele",
    refresh: "Yenile",
    close: "Kapat",
    tab_hex: "Bellek (Ctrl+1)",
    tab_editor: "Lua Betikleri (Ctrl+2)",
    tab_modify: "Düzenle (Ctrl+3)",
    hex_start: "Başlangıç",
    hex_no_base: "Süreç taban adresi (süreç seçilmedi)",
    length: "Uzunluk",
    bytes_fmt: "{} bayt",
    prev_page: "◀ Önceki",
    next_page: "Sonraki ▶",
    page_fmt: "Sayfa {}",
    reset_base: "Taban adrese dön",
    hex_pick_process: "Soldan bir süreç seç. Bellek görünümü modülün taban adresinden başlar.",
    reading: "Okunuyor…",
    read_failed_fmt: "Okuma başarısız: {}",
    script_dir: "Betik klasörü",
    reload: "Yeniden yükle",
    reloaded: "Diskten yeniden yüklendi",
    new_file_hint: "Yeni dosya adı",
    new_file: "Yeni",
    save: "Kaydet (Ctrl+S)",
    unsaved: "● Kaydedilmedi",
    scanned_fmt: "{} betik · {}",
    saved_fmt: "{} kaydedildi (overlay otomatik yeniden yükler)",
    save_failed_fmt: "Kaydedilemedi: {}",
    invalid_name: "Geçersiz dosya adı",
    file_exists: "Dosya zaten var",
    created_fmt: "{} oluşturuldu",
    create_failed_fmt: "Oluşturulamadı: {}",
    lock_table_fmt: "Sürücü kilitleri (Lua dahil){}",
    unlock_all: "Tüm kilitleri kaldır",
    lock_hint: "Bu penceredeki kilitleri aşağıdaki adres listesinden de düzenleyebilirsin.",
    no_locks: "Etkin kilit yok",
    lock_id: "Kilit kimliği",
    address: "Adres",
    source: "Kaynak",
    this_window: "Bu pencere",
    lua_other: "Lua / Diğer",
    unlock: "Kilidi kaldır",
    add: "Ekle",
    refresh_values: "Değerleri yenile",
    write_changes: "Değişiklikleri yaz",
    unfreeze_all: "Tüm sabitlemeleri kaldır",
    col_freeze: "Sabitle",
    col_type: "Tür",
    col_value: "Değer",
    no_process: "Süreç seçilmedi",
    delete: "Sil",
    type_i8: "1 bayt",
    type_i16: "2 bayt",
    type_i32: "4 bayt",
    type_i64: "8 bayt",
    type_u32: "4 bayt (işaretsiz)",
    type_f32: "Kayan nokta (32 bit)",
    type_f64: "Kayan nokta (64 bit)",
    status_initial: "Modül belleğini görmek için soldan bir süreç seç.",
    written: "Yazıldı",
    freezing: "Sabitleniyor",
    unfrozen: "Sabitleme kaldırıldı",
    all_unfrozen: "Tüm sabitlemeler kaldırıldı",
    unlocked: "Kilit kaldırıldı",
    process_list_fmt: "{} süreç",
    base_failed_fmt: "Modülün taban adresi okunamadı: {}",
    entries_fmt: "{} kayıt",
};

static EN: Strings = Strings {
    app_title: "KernelScript Inspector - Memory Tool",
    overlay_title: "KernelScript",
    show_fps: "Show FPS",
    ui_scale: "UI Scale",
    reset_ui_scale: "Reset to default",
    quit: "Quit",
    log: "Log",
    process: "Process",
    pid: "PID",
    filter_hint: "filter",
    refresh: "Refresh",
    close: "Close",
    tab_hex: "Memory (Ctrl+1)",
    tab_editor: "Lua Scripts (Ctrl+2)",
    tab_modify: "Modify (Ctrl+3)",
    hex_start: "Base",
    hex_no_base: "process base (no process selected)",
    length: "Length",
    bytes_fmt: "{} B",
    prev_page: "◀ Prev",
    next_page: "Next ▶",
    page_fmt: "Page {}",
    reset_base: "Reset",
    hex_pick_process: "Pick a process on the left; the hex view starts at its module base.",
    reading: "Reading...",
    read_failed_fmt: "read failed: {}",
    script_dir: "Script dir",
    reload: "Reload",
    reloaded: "Reloaded from disk",
    new_file_hint: "new file name",
    new_file: "New",
    save: "Save (Ctrl+S)",
    unsaved: "● unsaved",
    scanned_fmt: "{} scripts · {}",
    saved_fmt: "Saved {} (the overlay hot-reloads it)",
    save_failed_fmt: "save failed: {}",
    invalid_name: "invalid file name",
    file_exists: "file already exists",
    created_fmt: "created {}",
    create_failed_fmt: "create failed: {}",
    lock_table_fmt: "Driver lock table (incl. Lua){}",
    unlock_all: "Unlock all",
    lock_hint: "Locks from this window stay editable in the list below",
    no_locks: "No active locks",
    lock_id: "Lock ID",
    address: "Address",
    source: "Source",
    this_window: "this window",
    lua_other: "Lua/other",
    unlock: "Unlock",
    add: "Add",
    refresh_values: "Refresh",
    write_changes: "Write",
    unfreeze_all: "Unfreeze all",
    col_freeze: "Freeze",
    col_type: "Type",
    col_value: "Value",
    no_process: "no process selected",
    delete: "Del",
    type_i8: "1 byte",
    type_i16: "2 bytes",
    type_i32: "4 bytes",
    type_i64: "8 bytes",
    type_u32: "4 bytes (unsigned)",
    type_f32: "float",
    type_f64: "double",
    status_initial: "Pick a process on the left to browse its module memory",
    written: "written",
    freezing: "freezing",
    unfrozen: "unfrozen",
    all_unfrozen: "all unfrozen",
    unlocked: "unlocked",
    process_list_fmt: "processes: {}",
    base_failed_fmt: "module base failed: {}",
    entries_fmt: "{} entries",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_switches_tables() {
        set_lang(Lang::Tr);
        assert_eq!(s().refresh, "Yenile");
        set_lang(Lang::En);
        assert_eq!(s().refresh, "Refresh");
        set_lang(Lang::Tr);
        assert_eq!(s().refresh, "Yenile");
    }

    #[test]
    fn english_table_has_no_cjk() {
        let en = &EN;
        let all = [
            en.app_title,
            en.overlay_title,
            en.show_fps,
            en.quit,
            en.log,
            en.process,
            en.pid,
            en.filter_hint,
            en.refresh,
            en.close,
            en.tab_hex,
            en.tab_editor,
            en.tab_modify,
            en.hex_start,
            en.hex_no_base,
            en.length,
            en.bytes_fmt,
            en.prev_page,
            en.next_page,
            en.page_fmt,
            en.reset_base,
            en.hex_pick_process,
            en.reading,
            en.read_failed_fmt,
            en.script_dir,
            en.reload,
            en.reloaded,
            en.new_file_hint,
            en.new_file,
            en.save,
            en.unsaved,
            en.scanned_fmt,
            en.saved_fmt,
            en.save_failed_fmt,
            en.invalid_name,
            en.file_exists,
            en.created_fmt,
            en.create_failed_fmt,
            en.lock_table_fmt,
            en.unlock_all,
            en.lock_hint,
            en.no_locks,
            en.lock_id,
            en.address,
            en.source,
            en.this_window,
            en.lua_other,
            en.unlock,
            en.add,
            en.refresh_values,
            en.write_changes,
            en.unfreeze_all,
            en.col_freeze,
            en.col_type,
            en.col_value,
            en.no_process,
            en.delete,
            en.type_i8,
            en.type_i16,
            en.type_i32,
            en.type_i64,
            en.type_u32,
            en.type_f32,
            en.type_f64,
            en.status_initial,
            en.written,
            en.freezing,
            en.unfrozen,
            en.all_unfrozen,
            en.unlocked,
            en.process_list_fmt,
            en.base_failed_fmt,
            en.entries_fmt,
        ];
        for text in all {
            assert!(
                !text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                "{text:?} still contains CJK"
            );
        }
    }

    #[test]
    fn codes_round_trip() {
        assert_eq!(code(Lang::Tr), "tr");
        assert_eq!(code(Lang::En), "en");
    }
}
