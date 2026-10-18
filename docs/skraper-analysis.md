# Functional analysis of Skraper 1.4.2

> Historical record from version 0.1. Version 0.2 replaced ScreenScraper with SteamGridDB and added `gamelist.xml`. See the README for current behavior.

Inspected on 2026-10-18 using a local installation of Skraper 1.4.2. The release files and an XML composition descriptor were reviewed; the program was not run, and no code or credentials were extracted from the executable. This is an independent reproduction of the workflow documented and observed in those artifacts, not a translation of the internal code.

## Local evidence

- `SkraperUI.exe`, `SkraperLibrary2.dll`, .NET dependencies, and `SharpCompress.dll` are included in the distribution. The release notes mention Mono support and inspection of compressed files.
- `ReleaseNote.txt`, section 1.4.2: fixes for resizing and request filenames, optional CRC calculation, hash caching, and JSON conversion.
- Section 1.4: filtering `.cue`, `.m3u`, `.ccd`, and `.sub` files inside compressed archives.
- Earlier sections: regional priorities, request and media caches, API limits, game lists, composite images, videos, and manuals.
- `Mixes/3 Images Mix [HD][4-3].xml`: a 1280×960 viewport, a screenshot with alternatives, a wheel in the lower-right corner, and a 3D box in the lower-left. The XML includes anchoring, transparency, and fallback resources. This composition is not an artwork type offered by SteamGridDB and is not part of the current interface.

## Reproduced workflow

Select a system/folder → list ROMs → calculate identification data → query the catalog → choose a type/region → download → resize or compose → save PNG → show progress and preview.

The catalog and images come from ScreenScraper. The API game query receives application credentials, system, name, size, and hashes, and returns metadata and media. The implementation uses the JSON fields `response.jeu`, `noms`, and `medias`; authentication or quota errors stop the batch. Acceptance by the service must be confirmed with valid credentials.

## Scope comparison

| Feature | Rust project |
|---|---|
| Folder and system selection | GUI; one folder/system per batch |
| File identification | CRC32 + MD5 + SHA-1 |
| Compressed files | Basic ZIP support; multiple candidates require review |
| Media/region selection | Implemented with configurable order |
| Artwork offered by SteamGridDB | Grids, heroes, logos, and icons |
| Skraper screenshots and compositions | Not offered by SteamGridDB; unavailable in the app |
| Resizing | PNG, aspect ratio preserved, no cropping |
| Cache | Image bytes, not metadata or hashes |
| Service limits | One request at a time; stop when rate-limited |
| Interface | English, dark Breeze style, background worker |
| Game lists | EmulationStation and OnionOS |
| XML engine, videos, and manuals | Not implemented |

## Sources

- [Skraper: overview and features](https://skraper.net/index.html?lng=en)
- [ScreenScraper: API v2 documentation](https://www.screenscraper.fr/webapi2.php)

No ROMs, settings, or files in the original installation were modified.
