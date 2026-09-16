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
# Translator note: { $position } is the filtered count, { $total } is the total module count.
label-counter = { $position } / { $total }
label-copy-suffix = · copy { $count }
# Translator note: { $position } is the current match, { $total } is the total match count.
label-find-counter = { $position } / { $total }
empty-selection = Pick a module on the left to see its content
empty-filter = No modules match your search
empty-no-readable = No readable modules in this capture

## Input hints
hint-filter = Filter modules
hint-find = Text in this module
hint-find-keys = Enter next · Shift+Enter prev · Esc close

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
error-open-unreadable = Cannot read the file: { $detail }
error-open-dir = Is a directory, not a file
error-open-not-file = Not a regular file
error-open-large = File too large (>512 MiB)
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

## Settings
button-settings = Settings
button-settings-update = Settings — update { $version } is available
button-open-capture = Open another capture…
settings-title = Settings
settings-close = Close
settings-tab-general = General
settings-tab-updates = Updates
settings-tab-about = About
settings-theme = Theme
theme-system = System
theme-light = Light
theme-dark = Dark
settings-language = Language
settings-language-system = System language
update-checking = Checking for updates…
update-up-to-date = You are up to date
update-current-version = Version { $version }
update-current-build = build { $hash }
update-build-tooltip = Commit { $commit } · SHA-256 { $hash }
update-downloading = Downloading update…
update-verified = Verified with SHA-256
update-ready = Ready to install
update-install = Download and install
update-download-dmg = Download the .dmg
update-macos-hint = Drag the app to Applications to finish.
update-retry = Try again
update-open-page = Open the release page
update-never-checked = Not checked yet
update-last-checked = Last checked { $when }
update-changelog = What's new
time-just-now = just now
time-minutes-ago = { $count } min ago
time-hours-ago = { $count } h ago
time-days-ago = { $count } d ago
about-version = Version { $version }
about-copyright = Copyright © 2026 balakar94
about-license = Released under the MIT License.
about-license-link = View license
about-repository = Source code on GitHub
about-credits-heading = Credits
about-credits-body = Built with egui/eframe and other open-source Rust crates. Bundled typefaces: Inter, JetBrains Mono and Noto Sans SC, each under the SIL Open Font License 1.1; the full licence texts and third-party notices are installed with the app.
about-trademark = MikroTik RIF Viewer is an independent project, not affiliated with, endorsed by or sponsored by MikroTik. "MikroTik" and "RouterOS" are trademarks of their respective owner, used only to describe interoperability.
