# MikroTik RIF Viewer — Latviešu
# Trūkstošie identifikatori tiek ņemti no angļu faila.

app-title = MikroTik RIF Viewer

## Sveiciena ekrāns
welcome-blurb = Atveriet RouterOS atbalsta uztveri un lasiet visus tajā esošos moduļus. Viss notiek jūsu ierīcē — nekas netiek augšupielādēts.
welcome-start = Sākt

## Sākuma ekrāns
drop-title = Velciet savu supout.rif šeit
drop-title-hover = Atlaidiet failu, lai to atvērtu
drop-hint = vai noklikšķiniet, lai izvēlētos failu · viss notiek jūsu ierīcē

## Atvēršanas animācija
phase-reading = Faila lasīšana…
phase-indexing = Moduļu indeksēšana…
phase-preparing = Skata sagatavošana…
detail-indexing = Uztveres struktūras lasīšana
detail-read-of = { $done } no { $total }
detail-read = nolasīti { $size }
detail-modules-found = { $count ->
    [zero] atrasti { $count } moduļu
    [one] atrasts { $count } modulis
   *[other] atrasti { $count } moduļi
}

## Statuss un kājene
status-ready = Gatavs
status-decoding = Moduļa dekodēšana…
status-decoding-inline = Dekodē…
status-saved = Saglabāts { $path }
status-indexed = { $count ->
    [zero] indeksēti { $count } moduļu
    [one] indeksēts { $count } modulis
   *[other] indeksēti { $count } moduļi
}
status-copied = { $count ->
    [zero] nokopēti { $count } baitu starpliktuvē
    [one] nokopēts { $count } baits starpliktuvē
   *[other] nokopēti { $count } baiti starpliktuvē
}
footer-build = v{ $version } · { $os } · { $arch }

## Darbvieta
label-modules = Moduļi
label-line-numbers = Rindu numuri
label-no-capture = nav uztveres
label-compressed = { $size } saspiesti
label-counter = { $position } / { $total }
label-copy-suffix = · kopija { $count }
label-find-counter = { $position } / { $total }
empty-selection = Izvēlieties moduli kreisajā pusē, lai redzētu tā saturu
empty-filter = Neviens modulis neatbilst meklēšanai
empty-no-readable = Šajā uztverē nav lasāmu moduļu

## Ievades padomi
hint-filter = Filtrēt moduļus
hint-find = Teksts šajā modulī
hint-find-keys = Enter nākamais · Shift+Enter iepriekšējais · Esc aizvērt

## Pogas
button-open = Atvērt…
button-copy = Kopēt
button-save-as = Saglabāt kā…
button-previous = Iepriekšējais
button-next = Nākamais
button-clear = Notīrīt
button-toggle-rail = Rādīt vai paslēpt moduļu sarakstu
button-find = Meklēt šajā modulī

## Kļūdas un moduļa stāvoklis
module-unreadable = Šo moduli nevarēja indeksēt.
error-open = Nevarēja atvērt { $path }: { $reason }
error-open-unreadable = Failu nevar nolasīt: { $detail }
error-open-dir = Ir direktorija, nevis fails
error-open-not-file = Nav parasts fails
error-open-large = Fails ir pārāk liels (>512 MiB)
error-module = Nevarēja nolasīt moduli { $index }: { $reason }
error-save = Nevarēja saglabāt: { $reason }

## Sistēmas dialogi
dialog-open-title = Atvērt RouterOS uztveri
dialog-filter-name = RouterOS uztvere

## Atjauninājumi
update-available = Pieejams atjauninājums { $version }
update-check = Meklēt atjauninājumus
update-now = Atjaunināt tagad
update-later = Vēlāk
update-skip = Izlaist šo versiju
update-auto = Automātiski meklēt atjauninājumus

## Iestatījumi
button-settings = Iestatījumi
button-settings-update = Iestatījumi — pieejams atjauninājums { $version }
button-open-capture = Atvērt citu uztveri…
settings-title = Iestatījumi
settings-close = Aizvērt
settings-tab-general = Vispārīgi
settings-tab-updates = Atjauninājumi
settings-tab-about = Par
settings-theme = Tēma
theme-system = Sistēma
theme-light = Gaiša
theme-dark = Tumša
settings-language = Valoda
settings-language-system = Sistēmas valoda
update-checking = Notiek atjauninājumu meklēšana…
update-up-to-date = Jums ir jaunākā versija
update-current-version = Versija { $version }
update-current-build = būvējums { $hash }
update-build-tooltip = Commit { $commit } · SHA-256 { $hash }
update-downloading = Notiek atjauninājuma lejupielāde…
update-verified = Pārbaudīts ar SHA-256
update-ready = Gatavs instalēšanai
update-install = Lejupielādēt un instalēt
update-download-dmg = Lejupielādēt .dmg failu
update-macos-hint = Velciet lietotni uz mapi Applications, lai pabeigtu.
update-retry = Mēģināt vēlreiz
update-open-page = Atvērt laidiena lapu
update-never-checked = Vēl nav pārbaudīts
update-last-checked = Pēdējoreiz pārbaudīts { $when }
time-just-now = tikko
time-minutes-ago = pirms { $count } min
time-hours-ago = pirms { $count } st
time-days-ago = pirms { $count } d
about-version = Versija { $version }
about-copyright = Copyright © 2026 balakar94
about-license = Publicēts saskaņā ar MIT licenci.
about-license-link = Skatīt licenci
about-repository = Pirmkods vietnē GitHub
about-credits-heading = Pateicības
about-credits-body = Veidots ar egui/eframe un citām atklātā pirmkoda Rust bibliotēkām. Iekļautie burtveidoli: Inter, JetBrains Mono un Noto Sans SC, katrs saskaņā ar SIL Open Font License 1.1; pilnie licences teksti un trešo pušu paziņojumi tiek instalēti kopā ar lietotni.
about-trademark = MikroTik RIF Viewer ir neatkarīgs projekts, kas nav saistīts ar MikroTik, ko MikroTik neatbalsta un nereklamē. "MikroTik" un "RouterOS" ir to attiecīgo īpašnieku preču zīmes, kas izmantotas tikai savietojamības aprakstīšanai.
