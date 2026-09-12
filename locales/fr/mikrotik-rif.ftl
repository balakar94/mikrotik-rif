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
