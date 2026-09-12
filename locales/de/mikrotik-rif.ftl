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

## Eingabehinweise
hint-filter = Module filtern
hint-find = Text in diesem Modul

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
error-module = Modul { $index } konnte nicht gelesen werden: { $reason }
error-save = Speichern fehlgeschlagen: { $reason }

## Systemdialoge
dialog-open-title = RouterOS-Aufzeichnung öffnen
dialog-filter-name = RouterOS-Aufzeichnung
