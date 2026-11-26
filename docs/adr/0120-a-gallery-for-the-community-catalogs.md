# 0120. A gallery for the community catalogs: dates, popularity and plugin screenshots
Status: Accepted
Date: 2026-11-26
Accepted: 2026-11-26 (the maintainer's answers in the merger chat, on the gallery mockups of roadmap M5.5: "1 A, 2 şimdi, 3 evet, 4 görsel vs
ister", and "Katalog yayınlasın" for where popularity comes from). That is: option A (a Gallery screen), sorting by popular and new now,
plugin installs stay the owner's command, plugins get screenshots, and popularity is a number the signed catalog publishes.

Builds on ADR-0113 decision 6 (the theme catalog), ADR-0114 decision 7 (the signed plugin catalog), ADR-0024 (visual quality) and the
threat model rows T67 and T68. Changes the protocol (additive fields and one command) and both catalog index formats.

## Context
Both catalogs exist (`agora hub themes`, `agora hub plugins`), but the window shows them as plain lists: the theme catalog inside the
theme editor, the plugin catalog read-only inside the Plugins dialog. The architecture gap review (2026-11-26, section 3) asks for "a
gallery, not only a catalog: previews, sort by new or popular, and a one-step install". The indexes carry no date, no popularity and no
image, and both are read strictly (a field a hub does not know refuses the index or the entry).

Agora never sends anything out on its own: no telemetry, no install counts reported anywhere.

## Decision
1. **A Gallery screen in the window** (mockup option A): tabs Themes and Plugins, search, sort, and a grid of cards. A theme card draws
   the theme in its own colours (as Preview does today); a side panel shows it large with its schemes. A plugin card shows its name,
   author, licence, version, what it asks to see, its first screenshot, and its state. It opens from Settings (Look), the command palette
   and the Plugins dialog; the theme editor's catalog section becomes a link to it.
2. **Sorting.** Popular (the default when any entry has a popularity), New, Name, Author; installed ones are not moved. Ties fall back to
   the name.
3. **Popularity is published by the catalog, never counted by Agora.** Index format 2 gives each entry an optional `popularity`, a whole
   number from 0 to 1 000 000 000 that the catalog's maintainers set when they publish (for example the stars of the entry's source
   repository at release time). The window calls it "popularity, as the catalog publishes it". No machine counts or reports installs.
4. **New** is the optional `added`, a date `YYYY-MM-DD` the catalog sets when the entry first appears (not changed by later versions).
   A date after the day of the update is refused for that entry.
5. **Plugin screenshots.** Plugin index format 2 gives each entry up to 4 `screenshots`, each `{ "file", "sha256", "size", "alt" }`:
   a path under the index's folder ending in `.png` or `.webp` (the same path rules as the archive), at most 512 KiB, with an optional
   one-line `alt` of at most 200 characters. They are part of the signed index, so the signature covers their hashes. The hub fetches
   them during `agora hub plugins update` under the same deadlines, and keeps one only when its size and SHA-256 are the entry's, its
   first bytes are a PNG or WebP signature matching its extension, and its header says at most 2560 × 1600 pixels. At most 32 MiB of
   screenshots per catalog. A refused screenshot is dropped and named; the plugin stays. Themes get no images: a theme card is drawn from
   the theme itself.
6. **Both index formats go to 2**, and a hub reads 1 and 2. Format 2 adds only the optional fields above; everything else, and the
   strictness, stays. A hub older than this refuses a format-2 index whole ("update Agora"), as it already does for any newer format.
7. **Protocol, additive.** `ThemeCatalogEntry` and `PluginCatalogEntry` get `added` (string, empty when not given) and `popularity`
   (uint64, 0 when not given); `PluginCatalogEntry` gets `screenshots` (the alt texts, one per kept screenshot, in index order). A new
   command `PluginCatalogScreenshot { plugin_id, index }` answers `PluginCatalogScreenshotResult { media_type, data, alt }`
   (`image/png` or `image/webp`), refused with `plugin.catalog_off` while the catalog is off (a new error code) and `plugin.not_found` for
   an id or index it lacks. Any signed-in person may ask, as for the list.
8. **The window shows a screenshot as an image only.** It checks the first bytes again, builds a `data:` address with the fixed media
   type (the page policy already allows `img-src data:`; nothing else changes), draws it in an `<img>` with its alt text, and fetches it
   only when the card is on screen.
9. **Installing is unchanged.** A theme installs in one step for the person (as today). A plugin is installed by the hub's owner with
   `agora hub plugins install ID`; the gallery shows that command with a Copy button.
10. **Storage.** Hub database version 19: `added` and `popularity` columns on `theme_catalog` and `plugin_catalog`, and a
    `plugin_catalog_screenshot` table, all replaced in the same transaction as the catalog they belong to.

## Threat model
- A screenshot is untrusted content: only the browser decodes it, inside the page's sandboxing, never the hub or the node (the hub reads
  a few header bytes for the type and size). Size, count and pixel caps bound memory; the signature and the hash bind it to a reviewed
  catalog release.
- Popularity and dates are numbers and dates from the signed (plugins) or hashed-per-file (themes) catalog; they only order a list.
- Nothing is reported to anyone: there are no counts to leak.

## Alternatives
- **Count installs on the hub** (rejected by the maintainer): small numbers, and it shows people what their colleagues installed.
- **Images for themes**: unnecessary, the theme draws itself, and a picture could disagree with the file.
- **A one-step plugin install button for the owner**: needs a signed command; deferred (answer 3).

## Build order
1. This ADR, the schema (fields, command, golden fixtures), the docs' index formats.
2. The hub: both formats, the screenshots, migration 19, the command.
3. The window: the Gallery screen, the sorts, screenshots, the links from Settings, the palette and the Plugins dialog.
4. The terminal client is not changed (it has no catalog view).
