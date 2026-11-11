# terminal

The terminal view: libghostty-vt drawn by an Android `View`.

Soft-keyboard input uses normal text so compatible keyboards can offer swipe
typing. Composing text stays local until committed; personalized keyboard
learning is disabled by request to the IME.

- Ghostty source is pinned in the root flake to
  `0fa3d577a5614150ff8e2c152de5fb9d5c6d2dec`; no submodule initialization
  or network dependency fetching occurs in Gradle/Zig.
- `native/`: the JNI bridge in Zig (`src/jni.zig`), with `ghostty-vt` compiled
  in. Gradle stages the native and pinned Ghostty trees under `build/native-source`
  and runs Zig 0.17.0 with NDK 27.1.12297006 for arm64-v8a and x86_64,
  the only ABIs supported, into `libherdrterminal.so` (16 KB
  pages). `JNI_OnLoad` registers the natives, so the Zig functions keep short
  names and nothing else is exported.
- `src/main/kotlin`: the view, input and extra-keys bar.
- `vendor/fonts`: shipped as assets for what Android's fonts lack. Symbols
  Nerd Font Mono for Nerd glyphs; Noto Sans Symbols 2 (OFL), subset to the
  arrows, technical symbols (`⏵`), shapes, braille, dingbats and legacy
  computing blocks a TUI draws:
  `pyftsubset NotoSansSymbols2-Regular.ttf --unicodes=U+2190-21FF,U+2300-23FF,U+2460-24FF,U+25A0-25FF,U+2600-26FF,U+2700-27BF,U+27C0-27FF,U+2800-28FF,U+2900-297F,U+2B00-2BFF,U+1F780-1F7FF,U+1F800-1F8FF,U+1FB00-1FBFF --layout-features= --no-hinting --desubroutinize`.

The view and JNI code are a fork of
[expo-libghostty](https://github.com/arcboxlabs/expo-libghostty) 0.8.1 (MIT,
see `LICENSE`): its Android half, without the Expo/React Native wrapper, and
changed since for this app; its C++ JNI bridge became the Zig one. ghostty's
Zig API isn't stable yet, so moving the pin means checking `native/src/jni.zig`
against it.

Build from the repository root with `nix run --cores 2 --max-jobs 1 .#apk-debug`.
The Android Nix shell exposes `ZIG`, `GOATR_GHOSTTY_SOURCE` and
`GOATR_ZIG_PACKAGES`; Zig uses `--system` and upstream's hash-verified
transitive package graph. iroh independently retains NDK 28.2.13676358;
both SDK components are project-scoped Nix inputs. Developer checks are in
the root `AGENTS.md`.

## API

```kotlin
class TerminalView(context: Context) : LinearLayout {
    var onInput: ((ByteArray) -> Unit)?             // bytes for the PTY
    var onResize: ((cols: Int, rows: Int) -> Unit)? // after layout and on every change
    var onLink: ((String) -> Unit)?                 // Ctrl+tap or long-press "Open link" (OSC 8 or http(s) text)
    fun write(data: ByteArray, done: (() -> Unit)? = null)
    fun finish(exitCode: Int)                       // stop forwarding input
    fun destroy()                                   // free native state
    fun setFontSize(dp: Float)                      // default 14, clamped 4–64
    fun setTypeface(typeface: Typeface?)            // null: system monospace
    fun setLigatures(enabled: Boolean)
    fun setTheme(theme: TerminalTheme?)
    fun setKeyColors(colors: KeyColors?)
    fun setKeyBar(mode: KeyBarMode)                 // ALWAYS (default), TYPING, OFF
    fun setClipboardWrite(allowed: Boolean)         // programs copying (OSC 52); default on
    fun takeHardwareKey(event: KeyEvent): Boolean   // a keyboard key while focus is elsewhere
    val cellPixels: Pair<Int, Int>                  // cell size in px, for display facts
}
data class TerminalTheme(background, foreground, cursorColor,
    selectionBackground, selectionForeground, palette: List<String?>?)
data class KeyColors(background, key, foreground, activeKey, activeForeground)
```

Colors are strings in ghostty syntax (`#rrggbb`, X11 names).

**Threading:** main thread only. Every method must be called on it and every
callback fires on it. `write` parses synchronously and then calls `done`, so
grant credit for the bytes from `done`. Drawing happens on the next
Choreographer frame. Call `destroy()` when the view leaves for good.
