# MikroTik RIF Viewer — Deutsch
# Fehlende Kennungen werden aus der englischen Datei übernommen.

app-title = MikroTik RIF Viewer

## Willkommensbildschirm
welcome-blurb = Öffne eine RouterOS-Support-Aufzeichnung und lies alle enthaltenen Module. Alles läuft auf deinem Gerät — nichts wird hochgeladen.
welcome-start = Starten

## Startseite
drop-title = supout.rif hierher ziehen
drop-title-hover = Datei ablegen, um sie zu öffnen
drop-hint = oder klicken, um eine Datei zu wählen · alles läuft auf deinem Gerät

## Öffnungsanimation
phase-reading = Datei wird gelesen…
phase-indexing = Module werden indexiert…
phase-preparing = Ansicht wird vorbereitet…
detail-indexing = Struktur der Aufzeichnung wird gelesen
detail-read-of = { $done } von { $total }
detail-read = { $size } gelesen
detail-modules-found = { $count ->
    [one] { $count } Modul gefunden
   *[other] { $count } Module gefunden
}

## Status und Fußzeile
status-ready = Bereit
status-decoding = Modul wird dekodiert…
status-decoding-inline = Wird dekodiert…
status-saved = Gespeichert unter { $path }
status-indexed = { $count ->
    [one] { $count } Modul indexiert
   *[other] { $count } Module indexiert
}
status-copied = { $count ->
    [one] { $count } Byte in die Zwischenablage kopiert
   *[other] { $count } Bytes in die Zwischenablage kopiert
}
footer-build = v{ $version } · { $os } · { $arch }

## Arbeitsbereich
label-modules = Module
label-line-numbers = Zeilennummern
label-no-capture = keine Aufzeichnung
label-compressed = { $size } komprimiert
label-counter = { $position } / { $total }
label-copy-suffix = · Kopie { $count }
label-find-counter = { $position } / { $total }
empty-selection = Wähle links ein Modul, um seinen Inhalt zu sehen
empty-filter = Keine Module entsprechen deiner Suche
empty-no-readable = Keine lesbaren Module in dieser Aufzeichnung

## Eingabehinweise
hint-filter = Module filtern
hint-find = Text in diesem Modul
hint-find-keys = Enter weiter · Umschalt+Enter zurück · Esc schließen

## Schaltflächen
button-open = Öffnen…
button-copy = Kopieren
button-save-as = Speichern unter…
button-previous = Zurück
button-next = Weiter
button-clear = Leeren
button-toggle-rail = Modulliste ein- oder ausblenden
button-find = In diesem Modul suchen

## Fehler und Modulstatus
module-unreadable = Dieses Modul konnte nicht indexiert werden.
error-open = { $path } konnte nicht geöffnet werden: { $reason }
error-open-unreadable = Die Datei kann nicht gelesen werden: { $detail }
error-open-dir = Ist ein Verzeichnis, keine Datei
error-open-not-file = Keine reguläre Datei
error-open-large = Datei zu groß (>512 MiB)
error-module = Modul { $index } konnte nicht gelesen werden: { $reason }
error-save = Speichern fehlgeschlagen: { $reason }

## Systemdialoge
dialog-open-title = RouterOS-Aufzeichnung öffnen
dialog-filter-name = RouterOS-Aufzeichnung

## Updates
update-available = Version { $version } ist verfügbar
update-check = Nach Updates suchen
update-now = Jetzt aktualisieren
update-later = Später
update-skip = Diese Version überspringen
update-auto = Automatisch nach Updates suchen

## Einstellungen
button-settings = Einstellungen
button-settings-update = Einstellungen — Version { $version } ist verfügbar
button-open-capture = Weitere Aufzeichnung öffnen…
settings-title = Einstellungen
settings-close = Schließen
settings-tab-general = Allgemein
settings-tab-updates = Updates
settings-tab-about = Über
settings-theme = Design
theme-system = System
theme-light = Hell
theme-dark = Dunkel
settings-language = Sprache
settings-language-system = Systemsprache
update-checking = Suche nach Updates…
update-up-to-date = Du bist auf dem neuesten Stand
update-current-version = Version { $version }
update-current-build = Build { $hash }
update-build-tooltip = Commit { $commit } · SHA-256 { $hash }
update-downloading = Update wird heruntergeladen…
update-verified = Mit SHA-256 verifiziert
update-ready = Bereit zur Installation
update-install = Herunterladen und installieren
update-download-dmg = Die .dmg herunterladen
update-macos-hint = Ziehe die App zur Fertigstellung in den Ordner „Programme“.
update-retry = Erneut versuchen
update-open-page = Release-Seite öffnen
update-never-checked = Noch nicht geprüft
update-last-checked = Zuletzt geprüft { $when }
update-changelog = Neuigkeiten
time-just-now = gerade eben
time-minutes-ago = vor { $count } Min.
time-hours-ago = vor { $count } Std.
time-days-ago = vor { $count } T.
about-version = Version { $version }
about-copyright = Copyright © 2026 balakar94
about-license = Veröffentlicht unter der MIT-Lizenz.
about-license-link = Lizenz ansehen
about-repository = Quellcode auf GitHub
about-credits-heading = Danksagungen
about-credits-body = Erstellt mit egui/eframe und weiteren Open-Source-Rust-Crates. Mitgelieferte Schriftarten: Inter, JetBrains Mono und Noto Sans SC, jeweils unter der SIL Open Font License 1.1; die vollständigen Lizenztexte und Hinweise zu Drittanbietern werden mit der App installiert.
about-trademark = MikroTik RIF Viewer ist ein unabhängiges Projekt und steht in keiner Verbindung zu MikroTik, wird von MikroTik nicht unterstützt oder gesponsert. „MikroTik“ und „RouterOS“ sind Marken ihrer jeweiligen Inhaber und werden nur zur Beschreibung der Interoperabilität verwendet.
