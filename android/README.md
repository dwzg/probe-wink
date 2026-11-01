# ProbeWink Android app

Native Android app for the probe-wink modules, with home-screen widgets that wink in
one tap, no app opening needed.

- **App:** choose the module, all commands, alternate length, remote pairing, state, log.
- **Control widget (4×1):** LH, RH, Both, Alt, Stop, plus the last module state.
  Tap the status line to open the app.
- **Button widget (1×1):** one big button; you pick its action when you place it.
  Long-press it to change the action.

Android 12 or newer, Bluetooth LE. Kotlin, framework APIs only (no AndroidX, no
Google Play services).

## Build and install

```sh
cd android
./gradlew assembleDebug                       # app/build/outputs/apk/debug/app-debug.apk
./gradlew installDebug                        # phone on USB with USB debugging on
./gradlew testDebugUnitTest lintDebug         # protocol tests + lint
```

Needs the Android SDK (platform 36, build-tools 36.0.0) with its path in
`local.properties` (`sdk.dir=/home/you/Android/Sdk`), and a JDK 25 for Gradle
(`gradle/gradle-daemon-jvm.properties` makes Gradle pick it). The SDK can be installed
without Android Studio:

```sh
sdkmanager --sdk_root=$HOME/Android/Sdk "platform-tools" "platforms;android-36" "build-tools;36.0.0"
```

Without USB: copy the APK to the phone and open it (allow "install unknown apps" for
your file manager once). Debug and release builds are signed with this machine's debug
key, so updates install over each other only when built on the same machine.

## First run

1. Allow **Nearby devices** (Bluetooth) and **Notifications** (for the connection notice).
2. **Choose** → pick `PWink` from the paired devices, or **Scan for modules…** if it
   isn't paired yet. Only the LH (master) module accepts phone connections.
3. Pairing: Android asks for the module's passkey (`WINK_BLE_PASSKEY` the firmware was
   built with; for a devkit build it's in `firmware/.envrc`).
4. **Add widget**, or long-press the home screen → Widgets → ProbeWink.

## How it works

| Part | File | Role |
|---|---|---|
| Protocol | `Protocol.kt` | UUIDs, commands, parsing of state notifications; mirrors `firmware/wink-app/src/ble_phone.rs` and `firmware/wink-core/src/command.rs` |
| Link | `WinkLink.kt` | GATT client: connect by address, MTU 185, one operation at a time, notifications, pairing errors |
| Service | `WinkService.kt` | Foreground service (type `connectedDevice`) that owns the link. Widgets start it directly; it connects, sends, stays connected 90 s for follow-up taps, then disconnects |
| State | `Store.kt` | Settings, last known module state (persisted for the widgets), log |
| Widgets | `Widgets.kt`, `ActionWidgetConfigure.kt` | RemoteViews widgets, per-widget action |
| App | `MainActivity.kt` | Setup and full control |

Starting a foreground service from a widget tap is allowed on Android 12+ (user
interaction with a widget). The module is reached by its stored address, so no scan
is needed after setup.

## Not yet tested

Built, unit-tested (protocol) and lint-clean, but not yet run on a phone against a
module: connection, pairing prompts, widgets on a real launcher. Test against the
ESP32 devkit build first (see `firmware/README.md`).
