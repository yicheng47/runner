# Runner UI Design System & Component Guidelines

This document is the engineering reference for building interfaces in Runner. It defines the design tokens, elevation hierarchy, type scale, spacing rules, and component patterns implemented in `crates/runner-app/src/theme.rs` and `crates/runner-app/src/ui/`.

Before writing ad-hoc layout styles or raw pixel measurements for a new surface (such as settings, meters, or workspace panels), follow these conventions so the UI looks unified, adapts to all four themes, and scales smoothly with window zoom.

---

## 1. Core Principles

1. **Never use raw colors in UI code.**
   Never hardcode `rgb(...)`, hex literals, or un-themed colors. Every color must come from `theme::*` (e.g. `theme::bg()`, `theme::border()`, `theme::text()`). This guarantees that Carbon, Runner Light, Catppuccin Mocha, and Catppuccin Latte all render correctly.
2. **Scale with rems, not fixed px.**
   Runner allows users to zoom the app via `app_zoom`. In GPUI, text and layout dimensions should use `rems(n / 16.)` (or GPUI's rem-based utility methods like `p_4()`, `gap_2()`) rather than arbitrary `px(...)`. Fixed `px` values do not resize when the user adjusts zoom or rem size.
3. **Use existing UI components first.**
   Before hand-rolling divs for cards, headers, buttons, or badges, check `runner_app::ui::*`. Reusing `Card`, `SettingsCard`, `SettingsRow`, `Button`, `Badge`, and `PaneHeader` ensures consistent padding, border radius, and hover states.

---

## 2. Color & Elevation Tokens

Runner uses a tiered elevation model where surfaces sit on top of each other with increasing contrast:

| Token | Purpose | Typical Use Cases |
|---|---|---|
| `theme::bg()` | Base canvas | Window background, workspace canvas behind cards |
| `theme::panel()` | First elevation level | Content cards (`Card`, `SettingsCard`), popovers, modals |
| `theme::raised()` | Second elevation level | Segmented control tracks, inner well cards, input field backgrounds |
| `theme::sidebar()` | Chrome background | Left-hand navigation sidebar, status rail |
| `theme::sidebar_selected()` | Selected item fill | Active row in sidebar or active item in a list |

### Borders and Lines

| Token | Thickness | Purpose |
|---|---|---|
| `theme::border()` | `1px` (`border_1()`) | Default card outlines, panel separators, table dividers |
| `theme::border_strong()` | `1px` | Interactive controls (secondary/outline buttons, active focus rings) |
| `theme::sidebar_selected_border()` | `1px` | Outline around selected sidebar or list rows |

> [!TIP]
> When adding a subtle border inside a nested card or segment track, use `theme::border()`. For hover backgrounds or subtle highlight overlays, use `theme::with_alpha(theme::text(), 0.08)`.

### Typography Colors

| Token | Role | Usage |
|---|---|---|
| `theme::text()` | Primary foreground (`fg`) | Headings, titles, active labels, body text |
| `theme::muted()` | Secondary foreground (`fg_2`) | Subtitles, helper descriptions, inactive icons, units |
| `theme::faint()` | Tertiary foreground (`fg_3`) | Timestamps, empty state captions, disabled state labels |

### Semantic Accent & Feedback (`Tone`)

The `Tone` enum (`runner_app::ui::Tone`) maps semantic intent to theme colors:

| Token | Tone Variant | Meaning |
|---|---|---|
| `theme::accent()` | `Tone::Accent` | Primary actions, selected states, progress bars |
| `theme::accent_ink()` | — | Readable text color to place on top of `theme::accent()` fill |
| `theme::warning()` | `Tone::Warning` | Approaching limits, cautionary notices |
| `theme::danger()` | `Tone::Danger` | Destructive actions, quota exhausted, errors |
| `theme::info()` | `Tone::Info` | Informational callouts, active system status |

---

## 3. Typography Scale

Every font size in the app must use one of the centralized functions in `crate::theme`:

| Function | Pixel Equivalent | Purpose |
|---|---|---|
| `theme::text_micro()` | `9px` | Avatar initials, compact tag counters |
| `theme::text_caption()` | `10px` | Timestamps, badge labels, meter axis values |
| `theme::text_meta()` | `11px` | Secondary row metadata, table column headers, helper labels |
| `theme::text_ui()` | `12px` | Dense chrome, segmented control buttons, form hints |
| `theme::text_body()` | `13px` | Standard body copy, sidebar items, chat feed prompts |
| `theme::text_title()` | `14px` | Card headers, form input text, button labels |
| `theme::text_lead()` | `15px` | Modal dialog titles, drawer headers |
| `theme::text_heading()` | `16px` | Section headings (`settings_section_heading`) |
| `theme::text_display()` | `20px` | Surface page titles (`PaneHeader`) |
| `theme::text_display_lg()` | `24px` | Hero metric numbers (e.g. large point totals on a meter) |
| `theme::text_display_xl()` | `30px` | Single giant KPI figures |

Example usage:
```rust
div()
    .text_size(theme::text_title())
    .font_weight(FontWeight::SEMIBOLD)
    .text_color(theme::text())
    .child("Weekly Usage")
```

---

## 4. Spacing, Sizing & Radii

### Spacing Grid

Use 4px base increments expressed via rems or GPUI flex helpers:

- **4px** (`rems(4. / 16.)` or `.gap_1()`): Micro gaps between icon and text.
- **8px** (`rems(8. / 16.)` or `.gap_2()` / `.p_2()`): Standard item spacing in rows, card inner elements.
- **12px** (`rems(12. / 16.)` or `.gap_3()` / `.p_3()`): Form row padding, card body vertical rhythm.
- **16px** (`rems(1.)` or `.gap_4()` / `.p_4()`): Standard card padding, section gap.
- **20px–24px** (`rems(20. / 16.)`–`rems(24. / 16.)` or `.gap_5()` / `.gap_6()`): Gap between independent feature cards.

### Border Radii

- **Small interactive elements (buttons, inputs, stepper):** `rems(4. / 16.)` to `rems(6. / 16.)`.
- **Containers & Cards (`Card`, `SettingsCard`):** `rems(12. / 16.)`.
- **Badges & Pills:** `rounded_full()` (capsule shape) or `rems(4. / 16.)` for runtime tags.

---

## 5. Standard Component Catalog (`crates/runner-app/src/ui/`)

### Surface Headers (`SettingsHeader` / `PaneHeader`)
Use at the top of every distinct surface or settings page:
```rust
use runner_app::ui::PaneHeader;

PaneHeader::new("Usage & Limits", "Track point consumption and active provider windows.")
    .action(refresh_button)
```

### Cards & Sections
For custom panels and grouped metrics, use `Card` or recreate the standard card container:
```rust
use runner_app::ui::Card;

// Built-in standard card:
Card::new(content)

// Or custom layout matching standard card styling:
div()
    .rounded(rems(12. / 16.))
    .border_1()
    .border_color(theme::border())
    .bg(theme::panel())
    .p_4()
    .child(...)
```

### Settings Groups (`SettingsCard` & `SettingsRow`)
For structured key-value or setting rows with automatic divider lines:
```rust
use runner_app::ui::{SettingsCard, SettingsRow, Toggle};

SettingsCard::new(vec![
    SettingsRow::new("Show in Sidebar", toggle_widget)
        .subtitle("Display active usage badge in the project rail")
        .into_any_element(),
    SettingsRow::new("Auto-refresh", refresh_widget)
        .into_any_element(),
])
```

### Buttons (`Button`)
Use standard variants to maintain hierarchy:
- `ButtonVariant::Primary`: Main affirmative action (fill: `theme::accent()`, text: `theme::accent_ink()`).
- `ButtonVariant::Secondary` *(default)*: Standard action (bg: `theme::raised()`, border: `theme::border_strong()`).
- `ButtonVariant::Outline`: Border with transparent background.
- `ButtonVariant::Ghost`: Borderless action for icon buttons or row tools.
- `ButtonVariant::Danger`: Destructive actions (stops, deletions).

```rust
Button::new("refresh-btn", "Refresh")
    .icon("refresh-cw.svg")
    .size(ButtonSize::Sm)
    .variant(ButtonVariant::Secondary)
    .on_press(move |_, cx| { ... })
```

### Badges & Status Indicators
```rust
use runner_app::ui::{status_badge, Badge, Tone};

// Simple badge
Badge::new("Active", Tone::Accent)

// Badge with pulsing / status dot
status_badge("Connected", Tone::Accent)
status_badge("Quota Exhausted", Tone::Danger)
```

---

## 6. Recipes for New Feature Surfaces

### Recipe A: Metric / Meter Card
When building a metric display or progress meter (such as usage points, memory gauges, or token bars):

```rust
fn meter_card(title: &str, current: f64, max: f64, unit: &str) -> gpui::Div {
    let fraction = (current / max).clamp(0.0, 1.0) as f32;
    let is_warning = fraction > 0.8;
    let bar_color = if is_warning {
        theme::warning()
    } else {
        theme::accent()
    };

    div()
        .rounded(rems(12. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        // Card Header
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(theme::text_meta())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::muted())
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(format!("{current:.1} / {max:.0} {unit}")),
                ),
        )
        // Big Display Figure
        .child(
            div()
                .text_size(theme::text_display_lg())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(format!("{current:.1}")),
        )
        // Progress Meter Track & Bar
        .child(
            div()
                .w_full()
                .h(rems(6. / 16.))
                .rounded_full()
                .bg(theme::raised())
                .border_1()
                .border_color(theme::border())
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(gpui::percentage(fraction * 100.0))
                        .bg(bar_color)
                        .rounded_full(),
                ),
        )
}
```

### Recipe B: Segmented Control / Tab Switcher
For switching between units (e.g. Points vs Tokens or 24h vs 7d):

```rust
fn segmented_picker<T: Copy + PartialEq>(
    items: Vec<(T, &'static str)>,
    active: T,
    on_select: impl Fn(T, &mut gpui::App) + 'static + Clone,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(rems(2. / 16.))
        .p(rems(3. / 16.))
        .rounded(rems(8. / 16.))
        .bg(theme::raised())
        .border_1()
        .border_color(theme::border())
        .children(items.into_iter().map(|(val, label)| {
            let is_active = val == active;
            let cb = on_select.clone();
            div()
                .px(rems(8. / 16.))
                .py(rems(4. / 16.))
                .rounded(rems(5. / 16.))
                .text_size(theme::text_ui())
                .cursor_pointer()
                .when(is_active, |s| {
                    s.bg(theme::panel())
                        .border_1()
                        .border_color(theme::border())
                        .text_color(theme::text())
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!is_active, |s| {
                    s.text_color(theme::muted())
                        .hover(|h| h.text_color(theme::text()))
                })
                .on_click(move |_, _, cx| cb(val, cx))
                .child(label)
        }))
}
```

---

## 7. PR Review Checklist for UI Additions

When reviewing a PR that introduces new UI surfaces:

- [ ] **No raw hex/rgb:** All backgrounds, strokes, and text colors call `theme::*()`.
- [ ] **Zoom scaling:** Sizing and paddings use `rems(...)` or standard GPUI helpers rather than fixed `px(...)`.
- [ ] **Border consistency:** Outlines use `1px` stroke with `theme::border()` (or `theme::border_strong()` on active inputs/buttons).
- [ ] **Hierarchy alignment:** Main card surfaces use `theme::panel()`; sub-wells/tracks use `theme::raised()`.
- [ ] **Typography scale:** All text sizes are selected from `theme::text_*()`.
- [ ] **Theme tested:** Verified that the surface looks legible in both dark (`Carbon`) and light (`RunnerLight`) themes.
