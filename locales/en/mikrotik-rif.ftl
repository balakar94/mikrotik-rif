# MikroTik RIF Viewer — English (base locale)
# Every key used by the interface lives here. Other locales fall back to this
# file for any identifier they do not define.

app-title = MikroTik RIF Viewer

## Welcome screen
welcome-blurb = Open a RouterOS support capture and read every module it contains. Everything runs on your device — nothing is uploaded.
welcome-start = Start

## Home
drop-title = Drag your supout.rif here
drop-title-hover = Drop the file to open it
drop-hint = or click to choose a file · everything runs on your device

## Opening animation
phase-reading = Reading file…
phase-indexing = Indexing modules…
phase-preparing = Preparing view…
detail-indexing = Reading the capture structure
detail-read-of = { $done } of { $total }
detail-read = { $size } read
detail-modules-found = { $count ->
    [one] { $count } module found
   *[other] { $count } modules found
}

## Status and footer
status-ready = Ready
status-decoding = Decoding module…
status-decoding-inline = Decoding…
status-saved = Saved to { $path }
status-indexed = { $count ->
    [one] Indexed { $count } module
   *[other] Indexed { $count } modules
}
status-copied = { $count ->
    [one] Copied { $count } byte to the clipboard
   *[other] Copied { $count } bytes to the clipboard
}
footer-build = v{ $version } · { $os } · { $arch }

## Workspace labels
label-modules = Modules
label-line-numbers = Line numbers
label-no-capture = no capture
label-compressed = { $size } compressed
label-counter = { $position } / { $total }
label-copy-suffix = · copy { $count }
label-find-counter = { $position } / { $total }
empty-selection = Pick a module on the left to see its content
empty-filter = No modules match your search

## Input hints
hint-filter = Filter modules
hint-find = text in this module

## Buttons
button-open = Open…
button-copy = Copy
button-save-as = Save as…
button-previous = Previous
button-next = Next
button-clear = Clear
button-toggle-rail = Show or hide the module list
button-find = Search in this module

## Errors and module state
module-unreadable = This module could not be indexed.
error-open = Could not open { $path }: { $reason }
error-module = Could not read module { $index }: { $reason }
error-save = Could not save: { $reason }

## Native dialogs
dialog-open-title = Open a RouterOS capture
dialog-filter-name = RouterOS capture

## Updates
update-available = Update { $version } is available
update-check = Check for updates
update-now = Update now
update-later = Later
update-skip = Skip this version
update-auto = Automatic update checks
