# Dark Console Style Guide

## 1. Color System

Use semantic tokens as the source of truth.

```css
:root {
  --bg-app: #111214;
  --bg-sidebar: #2D2F33;
  --bg-surface: #222426;
  --bg-surface-alt: #1C1E21;

  --color-accent: #E34A3F;
  --color-accent-hover: #D94136;

  --color-text-primary: #F2F2F2;
  --color-text-secondary: #C8C8C8;
  --color-text-muted: #9B9B9B;

  --color-border: #2F3236;
  --color-disabled: #7C7C7C;
}
```

Rules:
- App canvas uses `--bg-app`.
- Navigation rail uses `--bg-sidebar`.
- Cards/panels use `--bg-surface` and `--bg-surface-alt`.
- Accent red is reserved for active/selected/primary actions and key status emphasis.
- Text on accent surfaces stays near-white.

## 2. Typography

Font direction:
- UI sans: `Inter`, with `Noto Sans`/`Roboto` fallbacks.
- Mono: `JetBrains Mono` for IDs, hashes, logs, and technical metadata.

**Bundling rule:** All font files ship with the Tauri binary. The UI
must never load fonts from a network source (no Google Fonts CDN, no
`@import url(...)`). hamm0r runs offline; the UI does too.

Type scale:
- App/page title: `28-32px`, semibold.
- Section title: `18-22px`, medium/semibold.
- Body labels and controls: `14-16px`, regular/medium.
- Metadata and helper text: `12-14px`, regular.
- Tabs/chips: `13-14px`, semibold.

Rules:
- Prioritize readability over stylistic thin weights.
- Use uppercase sparingly for utility labels and compact tabs.

## 3. Spacing and Layout

Spacing tokens:
- `--space-xs: 4px`
- `--space-sm: 8px`
- `--space-md: 12px`
- `--space-lg: 16px`
- `--space-xl: 20px`

Layout rules:
- Keep card/panel spacing explicit via gaps and padding.
- Favor structure from spacing and borders over shadow depth.
- Preserve dense, modular dashboard grouping.

## 4. Radius and Edge Language

- Standard radius: `8px` (`--radius`)
- Compact radius: `5px` (`--radius-sm`)

Rules:
- Rectangular-first components with subtle rounding.
- Avoid pills/capsules unless semantically useful (small tags/chips).

## 5. Component Treatment

Top chrome:
- Header bar may use accent red for strong orientation.
- Sidebar remains dark graphite with restrained hover states.

Cards and panels:
- Use dark surfaces with 1px subtle border.
- Avoid glossy effects and heavy shadowing.

Controls:
- Primary buttons use accent red.
- Secondary/ghost controls remain neutral gray.
- Active tabs use accent underline and subtle accent-tint background.
- Inactive controls use muted text and border contrast.

Motion/effects:
- Keep animation minimal and functional.
- No glassmorphism, no neon glow, no unnecessary gradients.