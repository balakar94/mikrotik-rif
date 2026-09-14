# MikroTik RIF Viewer — Français
# Les identifiants absents sont repris du fichier anglais.

app-title = MikroTik RIF Viewer

## Écran d'accueil
welcome-blurb = Ouvrez une capture de support RouterOS et consultez tous les modules qu'elle contient. Tout s'exécute sur votre appareil — rien n'est envoyé.
welcome-start = Démarrer

## Accueil
drop-title = Déposez votre supout.rif ici
drop-title-hover = Relâchez le fichier pour l'ouvrir
drop-hint = ou cliquez pour choisir un fichier · tout s'exécute sur votre appareil

## Animation d'ouverture
phase-reading = Lecture du fichier…
phase-indexing = Indexation des modules…
phase-preparing = Préparation de la vue…
detail-indexing = Lecture de la structure de la capture
detail-read-of = { $done } sur { $total }
detail-read = { $size } lus
detail-modules-found = { $count ->
    [one] { $count } module trouvé
   *[other] { $count } modules trouvés
}

## État et pied de page
status-ready = Prêt
status-decoding = Décodage du module…
status-decoding-inline = Décodage…
status-saved = Enregistré dans { $path }
status-indexed = { $count ->
    [one] { $count } module indexé
   *[other] { $count } modules indexés
}
status-copied = { $count ->
    [one] { $count } octet copié dans le presse-papiers
   *[other] { $count } octets copiés dans le presse-papiers
}
footer-build = v{ $version } · { $os } · { $arch }

## Espace de travail
label-modules = Modules
label-line-numbers = Numéros de ligne
label-no-capture = aucune capture
label-compressed = { $size } compressés
label-counter = { $position } / { $total }
label-copy-suffix = · copie { $count }
label-find-counter = { $position } / { $total }
empty-selection = Choisissez un module à gauche pour voir son contenu
empty-filter = Aucun module ne correspond à votre recherche

## Aides de saisie
hint-filter = Filtrer les modules
hint-find = texte dans ce module

## Boutons
button-open = Ouvrir…
button-copy = Copier
button-save-as = Enregistrer sous…
button-previous = Précédent
button-next = Suivant
button-clear = Effacer
button-toggle-rail = Afficher ou masquer la liste des modules
button-find = Rechercher dans ce module

## Erreurs et état du module
module-unreadable = Ce module n'a pas pu être indexé.
error-open = Impossible d'ouvrir { $path } : { $reason }
error-module = Impossible de lire le module { $index } : { $reason }
error-save = Échec de l'enregistrement : { $reason }

## Boîtes de dialogue système
dialog-open-title = Ouvrir une capture RouterOS
dialog-filter-name = Capture RouterOS

## Mises à jour
update-available = La version { $version } est disponible
update-check = Rechercher des mises à jour
update-now = Mettre à jour
update-later = Plus tard
update-skip = Ignorer cette version
update-auto = Vérifier les mises à jour automatiquement

## Paramètres
button-settings = Paramètres
button-settings-update = Paramètres — la version { $version } est disponible
button-open-capture = Ouvrir une autre capture…
settings-title = Paramètres
settings-close = Fermer
settings-tab-general = Général
settings-tab-updates = Mises à jour
settings-tab-about = À propos
settings-theme = Thème
theme-system = Système
theme-light = Clair
theme-dark = Sombre
settings-language = Langue
settings-language-system = Langue du système
update-checking = Recherche de mises à jour…
update-up-to-date = Vous êtes à jour
update-current-version = Version { $version }
update-current-build = compilation { $hash }
update-build-tooltip = Commit { $commit } · SHA-256 { $hash }
update-downloading = Téléchargement de la mise à jour…
update-verified = Vérifié avec SHA-256
update-ready = Prêt à installer
update-install = Télécharger et installer
update-download-dmg = Télécharger le .dmg
update-macos-hint = Faites glisser l'application dans Applications pour terminer.
update-retry = Réessayer
update-open-page = Ouvrir la page de la version
update-never-checked = Pas encore vérifié
update-last-checked = Dernière vérification { $when }
time-just-now = à l'instant
time-minutes-ago = il y a { $count } min
time-hours-ago = il y a { $count } h
time-days-ago = il y a { $count } j
about-version = Version { $version }
about-copyright = Copyright © 2026 balakar94
about-license = Publié sous licence MIT.
about-license-link = Voir la licence
about-repository = Code source sur GitHub
about-credits-heading = Crédits
about-credits-body = Créé avec egui/eframe et d'autres crates Rust open source. Polices incluses : Inter, JetBrains Mono et Noto Sans SC, chacune sous la SIL Open Font License 1.1 ; les textes complets des licences et les mentions des tiers sont installés avec l'application.
about-trademark = MikroTik RIF Viewer est un projet indépendant, sans lien d'affiliation, d'approbation ou de parrainage avec MikroTik. « MikroTik » et « RouterOS » sont des marques de leurs propriétaires respectifs, utilisées uniquement pour décrire l'interopérabilité.
