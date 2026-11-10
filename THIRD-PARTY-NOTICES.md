# Third-party notices

Goatr's independent code is GPL-3.0-or-later (`LICENSE`). Imported Zagal code
is Apache-2.0 (`ZAGAL-LICENSE`), Copyright 2026 Osc100. It includes the following.
Their license texts are in `app/src/main/assets/licenses/` (and, for the
terminal's fonts, `terminal/vendor/fonts/`), which are packaged into the app.

## Native terminal library (`libherdrterminal.so`)

| Component | License | Text |
| --- | --- | --- |
| [ghostty](https://github.com/ghostty-org/ghostty) (libghostty-vt, pinned Nix source) | MIT | `ghostty-LICENSE.txt` |
| [expo-libghostty](https://github.com/arcboxlabs/expo-libghostty) 0.8.1, the starting point for `terminal/` | MIT | `expo-libghostty-LICENSE.txt` (also `terminal/LICENSE`) |
| [Wuffs](https://github.com/google/wuffs), via ghostty | MIT or Apache-2.0 | `wuffs-LICENSE-MIT.txt` |
| [uucode](https://github.com/jacobsandlund/uucode), via ghostty, with Unicode data | MIT; Unicode License v3 | `uucode-LICENSE.md` |
| [Zig](https://ziglang.org) standard library | MIT | `zig-LICENSE.txt` |

## Libraries

| Component | License | Text |
| --- | --- | --- |
| [JSch](https://github.com/mwiede/jsch) (mwiede fork), with JZlib and jBCrypt | BSD-3-Clause; BSD; ISC | `jsch-LICENSE*.txt` |
| [Bouncy Castle](https://www.bouncycastle.org) (bcprov) | MIT-style | `bouncycastle-LICENSE.md` |
| [AndroidSVG](https://github.com/BigBadaboom/androidsvg) | Apache-2.0 | `Apache-2.0.txt` |
| AndroidX, Jetpack Compose, Material 3, Kotlin, kotlinx (coroutines, serialization, io) | Apache-2.0 | `Apache-2.0.txt` |
| CameraX (AndroidX), ZXing | Apache-2.0 | `Apache-2.0.txt` |
| iroh and native transport dependencies | MIT / Apache-2.0 and applicable dependency licenses | root `NOTICE`, `third_party/iroh-ffi/` provenance and Nix native-output notices |

## Fonts

| Font | License | Text |
| --- | --- | --- |
| [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) | OFL-1.1 | `jetbrains-mono-OFL.txt` |
| [Geist, Geist Mono](https://github.com/vercel/geist-font) | OFL-1.1 | `geist-OFL.txt` |
| [Symbols Nerd Font Mono](https://www.nerdfonts.com) | MIT (patcher); glyphs under their sources' licenses | `terminal/vendor/fonts/NERD-FONTS-LICENSE` |
| [Noto Sans Symbols 2](https://github.com/notofonts/symbols) (subset) | OFL-1.1 | `terminal/vendor/fonts/NOTO-SYMBOLS2-LICENSE` |

## Icons and sounds

| Asset | License | Text |
| --- | --- | --- |
| [Lucide](https://lucide.dev) glyphs (`assets/lucide`) | ISC | `icons-LICENSE-Lucide.txt` |
| Program icons (`assets/icons`), from herdr-app: [Simple Icons](https://simpleicons.org) and [LobeHub icons](https://github.com/lobehub/lobe-icons) | CC0-1.0; MIT | `icons-LICENSES.md`, `icons-LICENSE-LobeHub.txt` |
| Alert sounds (`res/raw`), from herdr-app | Apache-2.0 | `ZAGAL-LICENSE` |

Brand names and logos are their owners' trademarks; these licenses grant no
trademark rights.
