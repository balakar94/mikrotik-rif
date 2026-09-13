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

## Ievades padomi
hint-filter = Filtrēt moduļus
hint-find = teksts šajā modulī

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
