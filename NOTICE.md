# Notices

The native UI (`src/ui/`) recreates the interface of the previous React
frontend 1:1. That frontend — and therefore this implementation — reuses and
adapts material from the following open-source projects: design tokens,
component patterns, theme values, icon path data, and animation behavior.
Their licenses are retained below.

| Upstream | License | Adapted into |
| --- | --- | --- |
| [Lucide](https://lucide.dev) (`lucide-react`) | ISC | `src/ui/icons.rs` — SVG path data (Slack, hash, person, chat, …) |
| [Once UI](https://once-ui.com) (`@once-ui-system/core`) | MIT | `src/ui/components.rs`, `src/ui/theme.rs`, `src/ui/text.rs` — component patterns, tokens, typography; its Heroicons-outline icon set in `src/ui/icons.rs` |
| [Heroicons](https://heroicons.com) (via Once UI's icon set) | MIT | `src/ui/icons.rs` — SVG path data |
| [Motion Primitives](https://motion-primitives.com) | MIT | `src/ui/anim.rs`, `src/ui/shaders.rs` — `TextEffect` (per-word fade-in-blur), `AnimatedNumber` (spring count), `Spotlight` behavior |
| [motion](https://motion.dev) (framer-motion) | MIT | `src/ui/anim.rs` — the spring/easing/stagger semantics those effects replicate |
| [Tailwind CSS](https://tailwindcss.com) | MIT | `src/ui/theme.rs`, `src/ui/text.rs` — spacing, radii, and color token values derived from its generated CSS |

The Rust dependencies (egui/eframe/wgpu/winit, …) are crates.io packages
listed in `Cargo.lock` under their own licenses (MIT/Apache-2.0); they are
linked, not vendored, so their notices are not reproduced here.

## Lucide — ISC License

Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2023 as part
of Feather (MIT). All other copyright (c) for Lucide are held by Lucide
Contributors 2025.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

---

The MIT License (MIT) (for portions derived from Feather)

Copyright (c) 2013-2023 Cole Bemis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Once UI, Heroicons, Motion Primitives, motion, Tailwind CSS — MIT License

- Once UI (`@once-ui-system/core`) — Copyright (c) 2024-2026 Dopler
- Heroicons — Copyright (c) Tailwind Labs, Inc.
- Motion Primitives — Copyright (c) 2024 ibelick
- motion — Copyright (c) 2024 Motion B.V.
- Tailwind CSS — Copyright (c) Tailwind Labs, Inc.

The MIT License (MIT)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
