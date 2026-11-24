# Privacy

MikroTik RIF Viewer is a local desktop application. It has no accounts, no
telemetry and no analytics, and the project does not operate a server.

## Capture content

A capture (`supout.rif`) is read from the path you open and decoded inside the
process. Its content is never uploaded, never sent to the project, and never
included in a network request. The parser itself touches no files; the headless
CLI writes extracted modules only to the directory or standard output you name.

## The one network request

While automatic update checks are enabled (the default), the app asks GitHub on
each launch whether a newer release exists:

```text
GET https://api.github.com/repos/balakar94/mikrotik-rif/releases/latest
Accept: application/vnd.github+json
User-Agent: mikrotik-rif/<version>
If-None-Match: <ETag from the previous response>   # second and later checks
```

The request carries no capture data, no identifier and no usage information: the
only app-specific value is its name and version in the `User-Agent`. The trimmed
`ETag` from the previous response is cached so an unchanged release answers
`304` instead of consuming the unauthenticated GitHub API quota. A manual check
from Settings makes the same request.

GitHub receives the request, including your IP address and the standard HTTP
headers, under [GitHub's General Privacy
Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
The project does not receive or store that traffic.

If you accept an update, the installer and `SHA256SUMS.txt` (plus the optional
`.minisig`) are downloaded only from `github.com`, `api.github.com`,
`objects.githubusercontent.com` or `release-assets.githubusercontent.com`.
Every redirect hop is re-checked against that list, so a compromised response
cannot point the download at another host.

**To turn the check off:** Settings → Updates → clear **Automatic update
checks**. The app then works fully offline, apart from a manual check you start
yourself. Blocking network access for the app at the OS or firewall level is
also safe: nothing else in the product needs a socket.

## What is stored locally

- **Preferences:** theme, interface language and the automatic-update toggle.
- **Window geometry:** size and position of the main window.
- **Update state:** the time of the last check, the cached `ETag` and the
  version you chose to skip.
- **Panic log (only after a crash):** `mikrotik-rif-panic.log` in the system
  temporary directory. It holds the panic message and location, is capped at
  256 KiB, and is created with owner-only permissions on Unix. A pre-existing
  symlink at that path is moved aside instead of being followed.
- **Downloaded installers:** the system temporary directory on Windows (the
  installer is a throwaway launcher), `~/Downloads` on macOS and Linux when that
  directory exists (otherwise the temporary directory). An AppImage update
  additionally replaces the running image in place.

Preferences and update state are written by the GUI framework into the
platform's application-data directory under the app id
`io.github.balakar94.mikrotik-rif`: `$XDG_DATA_HOME/io.github.balakar94.mikrotik-rif`
(or `~/.local/share/…`) on Linux, `~/Library/Application Support/…` on macOS and
`%APPDATA%\io.github.balakar94.mikrotik-rif\data` on Windows. Nothing is synced
or uploaded.

## Contact

The project maintainer is the controller for the processing described here.
Questions or corrections: open an issue at
<https://github.com/balakar94/mikrotik-rif/issues>. This notice describes the
1.x implementation factually; a formal legal review is still pending.
