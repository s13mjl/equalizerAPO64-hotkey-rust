//! English/German strings.
//!
//! The four `LangString`s declared by `Setup.nsi` (`VersionError`, `UCRTError`,
//! `SecCheckForUpdates`, `SecRemoveName`) are reproduced verbatim. The page
//! texts follow the NSIS Modern UI 2 defaults used by `Setup.nsi` (which does
//! not override any of them); the remaining messages are the direct Rust
//! equivalents of the NSIS messages.

use windows::Win32::Globalization::GetUserDefaultUILanguage;

use crate::version::full_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    German,
}

macro_rules! tr {
    ($self:expr, $en:expr, $de:expr) => {
        match $self {
            Language::English => $en,
            Language::German => $de,
        }
    };
}

impl Language {
    /// MUI selects the language from the user's UI language. English is the
    /// first/default language, German is the only translation shipped by
    /// `Setup.nsi`.
    pub fn from_system() -> Self {
        let lang_id = unsafe { GetUserDefaultUILanguage() };
        // Primary language ID for German is LANG_GERMAN (0x07).
        if lang_id & 0x03ff == 0x07 {
            Language::German
        } else {
            Language::English
        }
    }

    /// `LangString VersionError`
    pub fn version_error(&self, target_arch: &str, detected_arch: &str) -> String {
        match self {
            Language::English => format!(
                "This installer is only supposed to be run on {target_arch} Windows. Please use the {detected_arch} installer."
            ),
            Language::German => format!(
                "Dieses Installationsprogramm kann nur auf einem {target_arch}-Windows verwendet werden. Bitte nutzen Sie die {detected_arch}-Version."
            ),
        }
    }

    /// `LangString UCRTError`
    pub fn ucrt_error(&self) -> &'static str {
        tr!(
            self,
            "Your Windows installation is missing required updates to use this program. Please install remaining Windows updates or the Visual C++ Redistributable for Visual Studio 2015 - 2022.\n\nDo you want to download the Visual C++ Redistributable now?",
            "Ihrer Windows-Installation fehlen benötigte Updates, um dieses Programm zu verwenden. Bitte installieren Sie ausstehende Windows-Updates oder das Visual C++ Redistributable für Visual Studio 2015 - 2022.\n\nMöchten Sie jetzt das Visual C++ Redistributable herunterladen?"
        )
    }

    /// `LangString SecCheckForUpdates`
    pub fn check_for_updates(&self) -> &'static str {
        tr!(
            self,
            "Check for updates automatically",
            "Automatisch auf Updates prüfen"
        )
    }

    /// `LangString SecRemoveName`
    pub fn remove_configs_and_backups(&self) -> &'static str {
        tr!(
            self,
            "Remove configurations and registry backups",
            "Konfigurationen und Registrierungsbackups entfernen"
        )
    }

    /// NSIS caption is `Name "Equalizer APO ${VERSION}"` for both the
    /// installer and the uninstaller (the page title carries Setup/Uninstall).
    pub fn window_title(&self) -> String {
        full_name()
    }

    pub fn uninstall_window_title(&self) -> String {
        full_name()
    }

    pub fn welcome_title(&self) -> String {
        tr!(
            self,
            format!("Welcome to the {} Setup Wizard", full_name()),
            format!("Willkommen beim {} Setup-Assistenten", full_name())
        )
    }

    pub fn welcome_text(&self) -> String {
        tr!(
            self,
            format!(
                "This wizard will guide you through the installation of {name}.\n\nIt is recommended that you close all other applications before starting Setup. This will make it possible to update relevant system files without having to reboot your computer.\n\nClick Next to continue.",
                name = full_name()
            ),
            format!(
                "Dieser Assistent wird Sie durch die Installation von {name} führen.\n\nEs wird empfohlen, alle anderen Anwendungen zu schließen, bevor Sie mit der Installation beginnen. Dadurch wird es möglich, relevante Systemdateien zu aktualisieren, ohne den Computer neu starten zu müssen.\n\nKlicken Sie auf Weiter, um fortzufahren.",
                name = full_name()
            )
        )
    }

    pub fn license_title(&self) -> &'static str {
        tr!(self, "License Agreement", "Lizenzvereinbarung")
    }

    pub fn license_text(&self) -> String {
        tr!(
            self,
            format!(
                "Please review the license terms before installing {}.",
                full_name()
            ),
            format!(
                "Bitte lesen Sie die Lizenzbedingungen, bevor Sie {} installieren.",
                full_name()
            )
        )
    }

    pub fn license_accept(&self) -> &'static str {
        tr!(
            self,
            "I accept the terms in the License Agreement",
            "Ich akzeptiere die Bedingungen der Lizenzvereinbarung"
        )
    }

    pub fn directory_title(&self) -> &'static str {
        tr!(
            self,
            "Choose Install Location",
            "Installationsordner wählen"
        )
    }

    pub fn directory_text(&self) -> String {
        tr!(
            self,
            format!("Choose the folder in which to install {}.", full_name()),
            format!(
                "Wählen Sie den Ordner, in dem {} installiert werden soll.",
                full_name()
            )
        )
    }

    pub fn destination_folder(&self) -> &'static str {
        tr!(self, "Destination Folder", "Zielordner")
    }

    pub fn browse(&self) -> &'static str {
        tr!(self, "Browse...", "Durchsuchen...")
    }

    pub fn space_required(&self) -> &'static str {
        tr!(self, "Space required", "Benötigter Speicherplatz")
    }

    pub fn space_available(&self) -> &'static str {
        tr!(self, "Space available", "Verfügbarer Speicherplatz")
    }

    pub fn invalid_directory(&self) -> &'static str {
        tr!(
            self,
            "Please enter a valid absolute installation folder.",
            "Bitte geben Sie einen gültigen absoluten Installationsordner an."
        )
    }

    pub fn start_menu_title(&self) -> &'static str {
        tr!(self, "Choose Start Menu Folder", "Startmenü-Ordner wählen")
    }

    pub fn start_menu_text(&self) -> String {
        tr!(
            self,
            format!(
                "Choose a Start Menu folder for the {} shortcuts.",
                full_name()
            ),
            format!(
                "Wählen Sie einen Startmenü-Ordner für die Verknüpfungen von {}.",
                full_name()
            )
        )
    }

    pub fn start_menu_folder_label(&self) -> &'static str {
        tr!(self, "Start Menu folder:", "Startmenü-Ordner:")
    }

    pub fn no_shortcuts(&self) -> &'static str {
        tr!(
            self,
            "Don't create shortcuts",
            "Keine Verknüpfungen erstellen"
        )
    }

    pub fn components_title(&self) -> &'static str {
        tr!(self, "Select Components", "Komponenten wählen")
    }

    pub fn components_text(&self) -> &'static str {
        tr!(
            self,
            "Select the components you want to install; clear the components you do not want to install. Click Next when you are ready to continue.",
            "Wählen Sie die Komponenten aus, die Sie installieren möchten. Entfernen Sie die Markierung der Komponenten, die Sie nicht installieren möchten. Klicken Sie auf Weiter, wenn Sie fortfahren möchten."
        )
    }

    pub fn uninstall_components_text(&self) -> &'static str {
        tr!(
            self,
            "Select the components you want to uninstall; clear the components you do not want to uninstall. Click Uninstall when you are ready to continue.",
            "Wählen Sie die Komponenten aus, die Sie deinstallieren möchten. Entfernen Sie die Markierung der Komponenten, die Sie nicht deinstallieren möchten. Klicken Sie auf Deinstallieren, wenn Sie fortfahren möchten."
        )
    }

    pub fn installing_title(&self) -> &'static str {
        tr!(self, "Installing", "Installiere")
    }

    pub fn installing_text(&self) -> String {
        tr!(
            self,
            format!("Please wait while {} is being installed.", full_name()),
            format!(
                "Bitte warten Sie, während {} installiert wird.",
                full_name()
            )
        )
    }

    pub fn uninstalling_title(&self) -> &'static str {
        tr!(self, "Uninstalling", "Deinstalliere")
    }

    pub fn uninstalling_text(&self) -> String {
        tr!(
            self,
            format!("Please wait while {} is being uninstalled.", full_name()),
            format!(
                "Bitte warten Sie, während {} deinstalliert wird.",
                full_name()
            )
        )
    }

    pub fn finish_install_title(&self) -> String {
        tr!(
            self,
            format!("Completing the {} Setup Wizard", full_name()),
            format!("Der {} Setup-Assistent ist beendet", full_name())
        )
    }

    pub fn finish_install_text(&self) -> String {
        tr!(
            self,
            format!("{} has been installed on your computer.", full_name()),
            format!("{} wurde auf Ihrem Computer installiert.", full_name())
        )
    }

    pub fn finish_uninstall_title(&self) -> String {
        tr!(
            self,
            format!("Completing the {} Uninstall Wizard", full_name()),
            format!("Der {} Deinstallations-Assistent ist beendet", full_name())
        )
    }

    pub fn finish_uninstall_text(&self) -> String {
        tr!(
            self,
            format!(
                "{} has been successfully removed from your computer.",
                full_name()
            ),
            format!(
                "{} wurde erfolgreich von Ihrem Computer entfernt.",
                full_name()
            )
        )
    }

    pub fn finish_hint(&self) -> &'static str {
        tr!(
            self,
            "Click Finish to close this wizard.",
            "Klicken Sie auf Fertig stellen, um den Assistenten zu schließen."
        )
    }

    pub fn uninstall_welcome_title(&self) -> String {
        tr!(
            self,
            format!("Welcome to the {} Uninstall Wizard", full_name()),
            format!(
                "Willkommen beim {} Deinstallations-Assistenten",
                full_name()
            )
        )
    }

    pub fn uninstall_welcome_text(&self) -> String {
        tr!(
            self,
            format!(
                "This wizard will guide you through the removal of {} from your computer.\n\nClick Next to continue.",
                full_name()
            ),
            format!(
                "Dieser Assistent wird Sie durch die Deinstallation von {} von Ihrem Computer führen.\n\nKlicken Sie auf Weiter, um fortzufahren.",
                full_name()
            )
        )
    }

    pub fn confirm_title(&self) -> &'static str {
        tr!(self, "Confirm Uninstall", "Deinstallation bestätigen")
    }

    pub fn confirm_text(&self) -> String {
        tr!(
            self,
            format!(
                "Are you sure you want to completely remove {} and all of its components?",
                full_name()
            ),
            format!(
                "Sind Sie sicher, dass Sie {} und alle Komponenten entfernen möchten?",
                full_name()
            )
        )
    }

    pub fn confirm_hint(&self) -> String {
        tr!(
            self,
            format!(
                "If you really want to remove {}, click Uninstall.",
                full_name()
            ),
            format!(
                "Wenn Sie {} wirklich entfernen möchten, klicken Sie auf Deinstallieren.",
                full_name()
            )
        )
    }

    pub fn back(&self) -> &'static str {
        tr!(self, "Back", "Zurück")
    }

    pub fn next(&self) -> &'static str {
        tr!(self, "Next", "Weiter")
    }

    pub fn install(&self) -> &'static str {
        tr!(self, "Install", "Installieren")
    }

    pub fn uninstall(&self) -> &'static str {
        tr!(self, "Uninstall", "Deinstallieren")
    }

    pub fn cancel(&self) -> &'static str {
        tr!(self, "Cancel", "Abbrechen")
    }

    pub fn close(&self) -> &'static str {
        tr!(self, "Close", "Schließen")
    }

    pub fn finish(&self) -> &'static str {
        tr!(self, "Finish", "Fertig stellen")
    }

    pub fn details(&self) -> &'static str {
        tr!(self, "Show details", "Details anzeigen")
    }

    pub fn details_hide(&self) -> &'static str {
        tr!(self, "Hide details", "Details ausblenden")
    }

    pub fn reboot_now(&self) -> &'static str {
        tr!(self, "Reboot now", "Jetzt neu starten")
    }

    pub fn abort_title(&self) -> &'static str {
        tr!(self, "Exit Setup", "Setup beenden")
    }

    pub fn abort_install_text(&self) -> String {
        tr!(
            self,
            format!("Are you sure you want to quit {} Setup?", full_name()),
            format!(
                "Sind Sie sicher, dass Sie {} Setup beenden wollen?",
                full_name()
            )
        )
    }

    pub fn abort_uninstall_text(&self) -> String {
        tr!(
            self,
            format!("Are you sure you want to quit {} Uninstall?", full_name()),
            format!(
                "Sind Sie sicher, dass Sie {} Deinstallation beenden wollen?",
                full_name()
            )
        )
    }

    pub fn error_title(&self) -> &'static str {
        tr!(self, "Error", "Fehler")
    }

    pub fn install_failed(&self) -> &'static str {
        tr!(
            self,
            "The installation failed. See the details below.",
            "Die Installation ist fehlgeschlagen. Siehe Details unten."
        )
    }

    pub fn uninstall_failed(&self) -> &'static str {
        tr!(
            self,
            "The uninstallation failed. See the details below.",
            "Die Deinstallation ist fehlgeschlagen. Siehe Details unten."
        )
    }

    pub fn installation_aborted(&self) -> &'static str {
        tr!(
            self,
            "The installation was aborted.",
            "Die Installation wurde abgebrochen."
        )
    }

    pub fn uninstallation_aborted(&self) -> &'static str {
        tr!(
            self,
            "The uninstallation was aborted.",
            "Die Deinstallation wurde abgebrochen."
        )
    }

    pub fn not_installed(&self) -> &'static str {
        tr!(
            self,
            "Equalizer APO is not installed.",
            "Equalizer APO ist nicht installiert."
        )
    }

    pub fn staging_missing(&self) -> &'static str {
        tr!(
            self,
            "The installer payload is incomplete. The following files are missing next to Setup.exe:",
            "Die Installationsdateien sind unvollständig. Folgende Dateien fehlen neben Setup.exe:"
        )
    }

    pub fn license_missing(&self) -> &'static str {
        tr!(
            self,
            "License.txt is missing from the source root.",
            "License.txt fehlt im Quellverzeichnis."
        )
    }

    pub fn elevation_failed(&self) -> &'static str {
        tr!(
            self,
            "Equalizer APO Setup requires administrator privileges. Please start the installer again as administrator.",
            "Equalizer APO Setup benötigt Administratorrechte. Bitte starten Sie das Installationsprogramm erneut als Administrator."
        )
    }

    pub fn preparing(&self) -> &'static str {
        tr!(self, "Preparing...", "Vorbereiten...")
    }

    pub fn cancelling(&self) -> &'static str {
        tr!(self, "Cancelling...", "Wird abgebrochen...")
    }

    pub fn installing_files(&self) -> &'static str {
        tr!(self, "Installing files...", "Dateien werden installiert...")
    }

    pub fn removing_files(&self) -> &'static str {
        tr!(self, "Removing files...", "Dateien werden entfernt...")
    }

    pub fn done(&self) -> &'static str {
        tr!(self, "Done.", "Fertig.")
    }

    pub fn update_download_url(&self) -> &'static str {
        "https://aka.ms/vs/17/release/vc_redist.x64.exe"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_langstrings_match_nsis() {
        assert_eq!(
            Language::English.version_error("x64", "x86"),
            "This installer is only supposed to be run on x64 Windows. Please use the x86 installer."
        );
        assert_eq!(
            Language::German.version_error("x64", "ARM64"),
            "Dieses Installationsprogramm kann nur auf einem x64-Windows verwendet werden. Bitte nutzen Sie die ARM64-Version."
        );
        assert_eq!(
            Language::English.check_for_updates(),
            "Check for updates automatically"
        );
        assert_eq!(
            Language::German.check_for_updates(),
            "Automatisch auf Updates prüfen"
        );
        assert_eq!(
            Language::English.remove_configs_and_backups(),
            "Remove configurations and registry backups"
        );
        assert_eq!(
            Language::German.remove_configs_and_backups(),
            "Konfigurationen und Registrierungsbackups entfernen"
        );
        assert!(Language::English.ucrt_error().contains("\n\n"));
        assert!(Language::German.ucrt_error().contains("Möchten Sie jetzt"));
    }
}
