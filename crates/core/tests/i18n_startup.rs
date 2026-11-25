//! The language is chosen after the settings are loaded, and loading them
//! already translates default values. Those early texts must not fix the
//! catalog in English. In its own test binary because the language is
//! process-wide state.

use adbscan_core::i18n::{self, tr, Language};

#[test]
fn texts_translated_before_choosing_the_language_do_not_fix_it() {
    // What `Settings::load()` does before `main` calls `set_language`.
    let _ = adbscan_core::settings::Settings::default();
    assert_eq!(tr("Balanced"), "Balanced");

    assert!(i18n::set_language(Language::Spanish));
    assert_eq!(i18n::language(), Language::Spanish);
    assert_eq!(tr("Balanced"), "Equilibrado");
}
