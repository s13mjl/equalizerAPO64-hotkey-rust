//! Qt translations of the original UpdateChecker.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language { English, German, French, Chinese }

static LANGUAGE: OnceLock<Language> = OnceLock::new();

pub fn set_language(language: Language) { let _ = LANGUAGE.set(language); }

pub fn language() -> Language { *LANGUAGE.get().unwrap_or(&Language::English) }

pub fn system_language() -> Language {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    match unsafe { GetUserDefaultUILanguage() } & 0x3ff {
        0x07 => Language::German, 0x0c => Language::French, 0x04 => Language::Chinese, _ => Language::English,
    }
}

pub fn tr(text: &str) -> String {
    let map: &[(&str, &str)] = match language() {
        Language::English => return text.to_string(),
        Language::German => GERMAN, Language::French => FRENCH, Language::Chinese => CHINESE,
    };
    map.iter().find(|(s, _)| *s == text).map(|(_, t)| (*t).to_string()).unwrap_or_else(|| text.to_string())
}

pub fn tr_arg(source: &str, args: &[&str]) -> String {
    let mut text = tr(source);
    for (index, arg) in args.iter().enumerate() { text = text.replace(&format!("%{index}"), arg); }
    text
}

static GERMAN: &[(&str, &str)] = &[
    ("A newer version of Equalizer APO is available:", "Eine neuere Version von Equalizer APO ist verfügbar:"),
    ("Cancel", "Abbrechen"),
    ("Close", "Schließen"),
    ("Don't show message for failed update check again", "Meldung für fehlgeschlagene Updateprüfung nicht mehr anzeigen"),
    ("Error installing Update Checker", "Fehler beim Installieren von Update Checker"),
    ("Error uninstalling Update Checker", "Fehler beim Deinstallieren von Update Checker"),
    ("Error while checking for update", "Fehler beim Überprüfen auf Updates"),
    ("Error while reading response of update check", "Fehler beim Lesen der Antwort der Updateprüfung"),
    ("Go to website", "Zur Website gehen"),
    ("No", "Nein"),
    ("No update available", "Kein Update verfügbar"),
    ("OK", "OK"),
    ("Remind me later", "Später erinnern"),
    ("Skip this version", "Diese Version überspringen"),
    ("The installed version %0 of Equalizer APO is up to date", "Die installierte Version %0 von Equalizer APO ist aktuell"),
    ("The installed version %0 of Equalizer APO is up to date.", "Die installierte Version %0 von Equalizer APO ist aktuell."),
    ("Update available", "Update verfügbar"),
    ("Yes", "Ja"),
];

static FRENCH: &[(&str, &str)] = &[
    ("A newer version of Equalizer APO is available:", "Une nouvelle version d'Equalizer APO est disponible:"),
    ("Cancel", "Annuler"),
    ("Close", "Fermer"),
    ("Don't show message for failed update check again", "Ne plus afficher le message d'échec de la vérification de la mise à jour"),
    ("Error installing Update Checker", "Erreur pendant l'installation du vérificateur de mise à jour (Update Checker)"),
    ("Error uninstalling Update Checker", "Erreur pendant la désinstallation du vérificateur de mise à jour (Update Checker)"),
    ("Error while checking for update", "Erreur pendant la recherche de mise à jour"),
    ("Error while reading response of update check", "Erreur pendant la lecture de la réponse de la vérification de mise à jour"),
    ("Go to website", "Allez au site web"),
    ("No", "Non"),
    ("No update available", "Pas de mise à jour disponible"),
    ("OK", "OK"),
    ("Remind me later", "Me rappeler plus tard"),
    ("Skip this version", "Sauter cette version"),
    ("The installed version %0 of Equalizer APO is up to date.", "La version installée %0 d'Equalizer APO est à jour."),
    ("Update available", "Mise à jour disponible"),
    ("Yes", "Oui"),
];

static CHINESE: &[(&str, &str)] = &[
    ("A newer version of Equalizer APO is available:", "有一个新版本的 Equalizer APO 可用："),
    ("Cancel", "取消"),
    ("Close", "关闭"),
    ("Don't show message for failed update check again", "不再显示更新检查失败的消息"),
    ("Error installing Update Checker", "安装更新检查器时出错"),
    ("Error uninstalling Update Checker", "卸载更新检查器时出错"),
    ("Error while checking for update", "检查更新时出错"),
    ("Error while reading response of update check", "读取更新检查响应时出错"),
    ("Go to website", "访问网站"),
    ("No", "否"),
    ("No update available", "无可用更新"),
    ("OK", "确定"),
    ("Remind me later", "稍后提醒我"),
    ("Skip this version", "跳过此版本"),
    ("The installed version %0 of Equalizer APO is up to date", "安装的 Equalizer APO %0 已是最新版本。"),
    ("The installed version %0 of Equalizer APO is up to date.", "安装的 Equalizer APO %0 已是最新版本。"),
    ("Update available", "更新可用"),
    ("Yes", "是"),
];

