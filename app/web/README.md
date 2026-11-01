# probe-wink phone app

A single web page that controls the headlamp modules over Web Bluetooth. It
installs to the Android home screen as a PWA and works offline once loaded.
No build step: `index.html` holds all the HTML, CSS and JS.

| File                    | Purpose                                         |
|-------------------------|-------------------------------------------------|
| `index.html`            | the app                                         |
| `manifest.webmanifest`  | PWA manifest (relative `start_url` and `scope`) |
| `sw.js`                 | offline cache (bump `VERSION` after changes)    |
| `icon.svg`, `icon-*.png`| app icon (PNGs rendered from the SVG with `rsvg-convert`) |

## Browser support

Web Bluetooth only works over HTTPS or on `localhost`.

| Browser                                | Works |
|----------------------------------------|-------|
| Chrome / Edge on Android               | yes (the intended target) |
| Chrome / Edge on Windows, macOS, Linux, ChromeOS | yes |
| Samsung Internet, Opera (Chromium)     | probably; untested |
| Any browser on iOS / iPadOS (Safari, Chrome, Firefox) | no, Apple does not allow Web Bluetooth |
| Firefox (all platforms)                | no |

Third-party iOS browsers with their own Bluetooth bridge (e.g. Bluefy) might
work, but nobody has tried. In unsupported browsers the page says so and
offers the demo mode.

## Only the LH module

The phone talks to the **LH (master)** module only. It advertises as `PWink`
(GAP name `ProbeWink`) with the service `7a1e0001-5c0b-4e57-9b1a-70726f626577`.
The RH module follows the master over ESP-NOW and has no phone service.

## First connection: pairing

1. Open the page, tap **Connect**, pick `PWink` / `ProbeWink` in Chrome's list.
   Android may first ask Chrome for the "Nearby devices" (or Location)
   permission.
2. The page reads the state characteristic. The firmware requires an
   encrypted, authenticated (passkey) link, so Android shows its pairing
   dialog: enter the 6-digit passkey the firmware was built with
   (`WINK_BLE_PASSKEY`, see `firmware/wink-app/src/secrets.rs` and
   [firmware/README.md](../../firmware/README.md)). The passkey is not stored anywhere in this app.
3. After pairing, the page subscribes to state notifications and the controls
   unlock.

If Android never shows the passkey dialog and the connection fails with an
authentication error, Android probably still holds an old bond (for example
after a reflash with a new passkey, or after the module evicted it; it keeps
six). Remove the "ProbeWink" pairing in **Android Settings > Bluetooth** and
connect again. Pairing with "Just Works" gives a bond but no access.

## Using it

- **Wink LH / Wink RH / Wink both**: one wink (`WINK L`, `WINK R`, `WINK B`).
  LH and RH as seen from the driver's seat.
- **Alternate n s**: `ALT n`, n set with the stepper from 1 to 10 (the
  firmware caps it at 10 s anyway). After every alternation, finished or
  stopped, alternate is locked for 20 s; winks still work.
- **STOP**: all relays off at once, even mid-travel. Always available while
  connected.
- **Refresh**: `STATUS`, the module re-sends its state.
- **Pair remote**: `PAIR REMOTE`, the module scans 30 s for a BLE HID remote.
  **Forget remote**: `FORGET REMOTE`.

While a wink or alternation runs, only STOP is accepted; the page greys out
the other buttons, and so does a supply fault (`overvoltage` /
`undervoltage`). The firmware stays the authority: refusals come back as
`error: …` and are shown under the state.

The module deep-sleeps 15 min after the last command. It sends `asleep`, the
link drops, and it wakes when a headlamp UP/DOWN line changes. Then tap
**Reconnect**.

**Reconnect.** After a dropped link the page reconnects to the same device
without the chooser. After a reload it can do the same only where Chrome
offers `navigator.bluetooth.getDevices()`; otherwise Connect opens the chooser
again (no new pairing needed, the bond stays).

The **Log** panel at the bottom lists sent commands (`→`) and received
notifications (`←`).

## Demo mode

Open `index.html?demo=1` (or follow the link the page shows in an
unsupported browser). A simulated module answers the commands with the same
states, errors and timing defaults as the firmware (`wink-core`), and a demo
panel can trigger supply faults, remote link changes, a dropped link and
sleep. Demo mode never touches Bluetooth.

## Hosting on GitHub Pages

GitHub Pages can publish a branch's root (`/`) or `/docs` folder, not `/app/web`.

**Option A (simplest): publish the repository root.**

1. Push the repository to GitHub. On the Free plan, Pages needs a public
   repository, and publishing the root makes every file in it reachable
   under the Pages URL.
2. Repository **Settings > Pages**: Source "Deploy from a branch", branch
   `main`, folder `/ (root)`. Save.
3. After a minute the app is at
   `https://dwzg.github.io/probe-wink/app/web/` (note the trailing `/app/web/`).

Pages runs Jekyll over the branch by default. It ignores files starting with
`_` or `.`, which this app does not have; a `.nojekyll` file in the
repository root turns Jekyll off if it ever gets in the way.

**Option B: publish only `app/web/`** with a GitHub Actions workflow
(Settings > Pages > Source "GitHub Actions"), e.g.
`.github/workflows/pages.yml`:

```yaml
name: pages
on:
  push:
    branches: [main]
    paths: [app/web/**]
  workflow_dispatch:
permissions:
  contents: read
  pages: write
  id-token: write
jobs:
  deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deploy.outputs.page_url }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/upload-pages-artifact@v3
        with:
          path: app/web
      - id: deploy
        uses: actions/deploy-pages@v4
```

Then the URL is `https://dwzg.github.io/probe-wink/` (no `/app/web/`).

Both work: the manifest, service worker and all links are relative.

**Install:** open the URL in Chrome on Android, menu > "Add to home screen" /
"Install app".

**Updates:** the service worker serves the cached copy first and fetches a
fresh one in the background, so a new version shows on the second launch.
Bump `VERSION` in `sw.js` when you change any file so old caches are dropped.

## Local testing

```sh
cd app/web && python3 -m http.server 8000
# open http://localhost:8000/?demo=1
```

`localhost` counts as secure, so Web Bluetooth also works there in desktop
Chrome with a real module in range.

## Firmware interface used

| UI                | Written to `7a1e0002-…`    |
|-------------------|----------------------------|
| Wink LH / RH / both | `WINK L` / `WINK R` / `WINK B` |
| Alternate n s     | `ALT n` (n = 1…10)         |
| STOP              | `STOP`                     |
| Refresh           | `STATUS`                   |
| Pair / Forget remote | `PAIR REMOTE` / `FORGET REMOTE` |

Writes use `writeValueWithResponse`, one GATT operation at a time. The state
characteristic `7a1e0003-…` is read once on connect and then notified:
`asleep`, `idle`, `wink`, `alternating`, `resting`, `overvoltage`,
`undervoltage`, plus `error: <reason>` and `remote: <status>` replies.
Source: [`firmware/wink-app/src/ble_phone.rs`](../../firmware/wink-app/src/ble_phone.rs),
[`firmware/wink-core/src/command.rs`](../../firmware/wink-core/src/command.rs).

Status: tested in headless Chrome (demo mode, and the real code path against
a mocked `navigator.bluetooth`). Not yet tested against a module or on an
Android phone.
