//! Custom GPUI element painting the `alacritty_terminal` grid: cell
//! backgrounds as quads, glyphs as shaped lines, cursor on top.
//!
//! Alignment strategy: same-style ASCII spans are shaped as one run
//! (monospace advance == cell width, so columns stay true); custom
//! terminal graphics are painted procedurally; remaining non-ASCII is
//! shaped per cell and painted at its own column origin, so a fallback
//! font's advance can never skew the grid. gpui caches shaped lines,
//! so per-cell shaping of repetitive glyphs stays cheap.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Point as GridPoint, Side};
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{point_to_viewport, viewport_to_point, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui::{
    fill, outline, point, px, relative, size, AnyTooltip, AnyView, App, Bounds, ClipboardItem,
    ContentMask, Context, CursorStyle, DispatchPhase, Element, ElementInputHandler, Entity,
    FocusHandle, Font, FontFallbacks, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseButton as GpuiMouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, ShapedLine,
    SharedString, Style, Task, TextAlign, TextRun, UnderlineStyle, Window,
};

use runner_app::terminal_ime::TerminalInput;
use runner_app::terminal_resize::{
    size_push_verdict, terminal_grid_size, SizePushVerdict, TerminalGridSize,
};
use runner_app::theme;
use runner_app::ui::tooltip::tooltip_view;
use runner_terminal::mappings::{
    encode_mouse_motion, encode_mouse_press, encode_mouse_release, MouseButton, MouseModifiers,
};
use runner_terminal::palette::{self, TerminalPalette};
use runner_terminal::terminal::{LinkTarget, TerminalLink, TerminalMirror};

use super::glyphs::{snapped_cell_bounds, ProceduralCell};

/// A terminal row's height in pixels: the font size times the line-height
/// setting, rounded so rows land on whole pixels.
pub fn row_height(font_size: f32, line_height_factor: f32) -> f32 {
    (font_size * line_height_factor).round()
}

#[derive(Clone, Copy)]
struct TerminalGeometry {
    bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    cols: usize,
    rows: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalHit {
    point: GridPoint,
    side: Side,
    viewport_column: usize,
    viewport_row: usize,
}

#[derive(Clone)]
enum DragKind {
    Link {
        initial: GridPoint,
        target: LinkTarget,
        moved: bool,
    },
    Local {
        initial: TerminalHit,
        alt_click: bool,
        moved: bool,
        started_at: Instant,
        column_select: bool,
    },
    Reported {
        last: (usize, usize, MouseModifiers),
    },
}

#[derive(Clone)]
struct DragState {
    generation: u64,
    button: MouseButton,
    kind: DragKind,
    autoscroll: i32,
    endpoint_column: usize,
    endpoint_side: Side,
    rows: usize,
}

/// How long the pointer rests on a link before its tooltip appears; matches
/// gpui's own hover tooltips.
const LINK_TOOLTIP_DELAY: Duration = Duration::from_millis(500);

/// Half a blink cycle, as in Terminal.app and xterm.
const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// Only a focused terminal in the active window blinks, on one timer that
/// stops when the terminal is no longer painted. `restart` is the output
/// sequence, typed input and cursor position: any change shows the cursor
/// again for a full interval.
#[derive(Default)]
struct CursorBlink {
    task: Option<Task<()>>,
    running: bool,
    hidden: bool,
    painted: bool,
    next_toggle: Option<Instant>,
    restart: Option<CursorBlinkRestart>,
}

type CursorBlinkRestart = (u64, u64, GridPoint);

#[derive(Clone)]
struct LinkTooltip {
    target: LinkTarget,
    modifier_held: bool,
    view: AnyView,
    mouse_position: Point<Pixels>,
}

pub(crate) struct TerminalInteraction {
    session: Arc<TerminalMirror>,
    pub(crate) pane_size: Option<TerminalGridSize>,
    scroll_accumulator: f32,
    drag: Option<DragState>,
    next_generation: u64,
    hovered_link: Option<TerminalLink>,
    hover_signature: Option<(GridPoint, u64, usize, (u16, u16))>,
    hover_modifiers: Option<(bool, bool, bool, bool)>,
    link_modifier_held: bool,
    hover_since: Option<Instant>,
    link_tooltip: Option<LinkTooltip>,
    blink: CursorBlink,
}

impl TerminalInteraction {
    pub(crate) fn new(session: Arc<TerminalMirror>) -> Self {
        Self {
            session,
            pane_size: None,
            scroll_accumulator: 0.,
            drag: None,
            next_generation: 0,
            hovered_link: None,
            hover_signature: None,
            hover_modifiers: None,
            link_modifier_held: false,
            hover_since: None,
            link_tooltip: None,
            blink: CursorBlink::default(),
        }
    }

    /// Whether this frame draws the cursor.
    fn cursor_visible(
        &mut self,
        blinking: bool,
        restart: CursorBlinkRestart,
        cx: &mut Context<Self>,
    ) -> bool {
        if !blinking {
            self.blink = CursorBlink::default();
            return true;
        }
        self.blink.painted = true;
        let now = cx.background_executor().now();
        if !self.blink.running || self.blink.restart != Some(restart) {
            self.blink.hidden = false;
            self.blink.restart = Some(restart);
            self.blink.next_toggle = Some(now + CURSOR_BLINK_INTERVAL);
        }
        if !self.blink.running {
            self.blink.running = true;
            self.blink.task = Some(cx.spawn(async move |weak, cx| {
                while let Some(wait) = weak
                    .update(cx, |this, cx| this.blink_wait(cx))
                    .ok()
                    .flatten()
                {
                    cx.background_executor().timer(wait).await;
                }
            }));
        }
        !self.blink.hidden
    }

    /// How long the blink timer sleeps next, toggling the cursor when its
    /// interval is up; `None` stops it.
    fn blink_wait(&mut self, cx: &mut Context<Self>) -> Option<Duration> {
        if !self.blink.running {
            return None;
        }
        let now = cx.background_executor().now();
        let next = self.blink.next_toggle?;
        if now < next {
            return Some(next - now);
        }
        if !std::mem::take(&mut self.blink.painted) {
            self.blink.running = false;
            self.blink.hidden = false;
            return None;
        }
        self.blink.hidden = !self.blink.hidden;
        self.blink.next_toggle = Some(now + CURSOR_BLINK_INTERVAL);
        cx.notify();
        Some(CURSOR_BLINK_INTERVAL)
    }

    fn link_tooltip_visible(&self) -> bool {
        self.hovered_link.is_some()
            && self
                .hover_since
                .is_some_and(|since| since.elapsed() >= LINK_TOOLTIP_DELAY)
    }

    fn scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        geometry: TerminalGeometry,
        interactive: bool,
        window: &Window,
    ) -> bool {
        let lines = match event.delta {
            ScrollDelta::Lines(point) => point.y,
            ScrollDelta::Pixels(point) => f32::from(point.y) / f32::from(window.line_height()),
        };
        let whole = accumulate_scroll(&mut self.scroll_accumulator, lines);
        if whole == 0 {
            return false;
        }
        if interactive {
            let (column, row) = viewport_cell(geometry, event.position);
            self.session
                .scroll(whole, event.modifiers.shift, column, row);
        } else {
            self.session.scroll_local(whole);
        }
        true
    }

    fn link_tooltip(
        &mut self,
        target: LinkTarget,
        modifier_held: bool,
        mouse_position: Point<Pixels>,
        cx: &mut App,
    ) -> LinkTooltip {
        if let Some(cached) = self
            .link_tooltip
            .as_ref()
            .filter(|cached| cached.target == target && cached.modifier_held == modifier_held)
        {
            return cached.clone();
        }
        let content = crate::file_links::link_tooltip_content(&target, modifier_held, cx);
        let tooltip = LinkTooltip {
            target,
            modifier_held,
            view: tooltip_view(content, cx),
            mouse_position,
        };
        self.link_tooltip = Some(tooltip.clone());
        tooltip
    }

    fn refresh_hovered_link(
        &mut self,
        modifiers: gpui::Modifiers,
        geometry: TerminalGeometry,
        position: Point<Pixels>,
        hovered: bool,
        interactive: bool,
    ) -> bool {
        if !hovered {
            self.hover_signature = None;
            self.hover_modifiers = None;
            self.hover_since = None;
            self.link_tooltip = None;
            return self.hovered_link.take().is_some();
        }

        let modifier_signature = (
            modifiers.platform,
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
        );
        let modifiers_changed =
            self.hover_modifiers.replace(modifier_signature) != Some(modifier_signature);
        self.link_modifier_held = link_modifier(modifiers);
        if !interactive {
            self.hover_signature = None;
            self.hover_since = None;
            return self.hovered_link.take().is_some() || modifiers_changed;
        }

        let point = hit_test(&self.session, geometry, position).point;
        let activity = self.session.output_activity();
        let display_offset = self.session.scroll_state().display_offset;
        let signature = (
            point,
            activity.last_seq,
            display_offset,
            self.session.size(),
        );
        if self.hover_signature == Some(signature) {
            return modifiers_changed;
        }

        self.hover_signature = Some(signature);
        let hovered_link = self.session.link_at(point);
        let changed = hovered_link != self.hovered_link;
        if changed {
            self.hover_since = hovered_link.is_some().then(Instant::now);
        }
        self.hovered_link = hovered_link;
        changed || modifiers_changed
    }

    fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        geometry: TerminalGeometry,
        interactive: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(button) = mouse_button(event.button) else {
            return;
        };
        let hit = hit_test(&self.session, geometry, event.position);
        let mode = self.session.mode();
        let modifiers = mouse_modifiers(event.modifiers);
        let reporting =
            interactive && mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift;
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;

        if interactive && button == MouseButton::Left && link_modifier(event.modifiers) {
            if let Some(link) = self.session.link_at(hit.point) {
                self.drag = Some(DragState {
                    generation,
                    button,
                    kind: DragKind::Link {
                        initial: hit.point,
                        target: link.target,
                        moved: false,
                    },
                    autoscroll: 0,
                    endpoint_column: hit.point.column.0,
                    endpoint_side: hit.side,
                    rows: geometry.rows,
                });
                return;
            }
        }

        if reporting {
            if let Some(bytes) = encode_mouse_press(
                mode,
                button,
                hit.viewport_column,
                hit.viewport_row,
                modifiers,
                false,
            ) {
                let _ = self.session.write_user_bytes(&bytes);
            }
            self.drag = Some(DragState {
                generation,
                button,
                kind: DragKind::Reported {
                    last: (hit.viewport_column, hit.viewport_row, modifiers),
                },
                autoscroll: 0,
                endpoint_column: hit.point.column.0,
                endpoint_side: hit.side,
                rows: geometry.rows,
            });
            return;
        }

        match button {
            MouseButton::Left => {
                let column_select = if event.modifiers.shift {
                    let column_select = self.session.selection_type() == Some(SelectionType::Block);
                    self.session.update_selection(hit.point, hit.side);
                    column_select
                } else {
                    let ty = match event.click_count {
                        2 => SelectionType::Semantic,
                        count if count >= 3 => SelectionType::Lines,
                        _ if event.modifiers.alt => SelectionType::Block,
                        _ => SelectionType::Simple,
                    };
                    self.session.start_selection(ty, hit.point, hit.side);
                    ty == SelectionType::Block
                };
                self.drag = Some(DragState {
                    generation,
                    button,
                    kind: DragKind::Local {
                        initial: hit,
                        alt_click: event.modifiers.alt && interactive,
                        moved: false,
                        started_at: Instant::now(),
                        column_select,
                    },
                    autoscroll: 0,
                    endpoint_column: hit.point.column.0,
                    endpoint_side: hit.side,
                    rows: geometry.rows,
                });
                self.spawn_autoscroll(generation, cx);
            }
            MouseButton::Right => {
                if !self.session.selection_contains(hit.point)
                    && !self.session.cell_is_whitespace(hit.point)
                {
                    self.session
                        .start_selection(SelectionType::Semantic, hit.point, hit.side);
                }
            }
            MouseButton::Middle => {}
        }
    }

    fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        geometry: TerminalGeometry,
        interactive: bool,
    ) {
        let Some(mut drag) = self.drag.take() else {
            return;
        };
        if event.pressed_button.and_then(mouse_button) != Some(drag.button) {
            self.drag = Some(drag);
            return;
        }
        let mut hit = hit_test(&self.session, geometry, event.position);
        match &mut drag.kind {
            DragKind::Link { initial, moved, .. } => {
                *moved |= hit.point != *initial;
            }
            DragKind::Reported { last } => {
                let modifiers = mouse_modifiers(event.modifiers);
                let current = (hit.viewport_column, hit.viewport_row, modifiers);
                if interactive && *last != current {
                    if let Some(bytes) = encode_mouse_motion(
                        self.session.mode(),
                        drag.button,
                        hit.viewport_column,
                        hit.viewport_row,
                        modifiers,
                        false,
                    ) {
                        let _ = self.session.write_user_bytes(&bytes);
                    }
                }
                *last = current;
            }
            DragKind::Local {
                initial,
                moved,
                column_select,
                ..
            } => {
                drag.autoscroll = autoscroll_amount(geometry.bounds, event.position);
                if drag.autoscroll > 0 {
                    hit = viewport_edge_hit(&self.session, geometry, hit, *column_select, true);
                } else if drag.autoscroll < 0 {
                    hit = viewport_edge_hit(&self.session, geometry, hit, *column_select, false);
                }
                *moved |= hit.point != initial.point || hit.side != initial.side;
                drag.endpoint_column = hit.point.column.0;
                drag.endpoint_side = hit.side;
                drag.rows = geometry.rows;
                self.session.update_selection(hit.point, hit.side);
            }
        }
        self.drag = Some(drag);
    }

    fn mouse_up(
        &mut self,
        event: &MouseUpEvent,
        geometry: TerminalGeometry,
        interactive: bool,
        copy_on_select: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(button) = mouse_button(event.button) else {
            return;
        };
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.button != button {
            self.drag = Some(drag);
            return;
        }
        let hit = hit_test(&self.session, geometry, event.position);
        match drag.kind {
            DragKind::Link {
                target,
                moved: false,
                ..
            } if interactive
                && link_modifier(event.modifiers)
                && self
                    .session
                    .link_at(hit.point)
                    .is_some_and(|link| link.target == target) =>
            {
                match target {
                    LinkTarget::Url(uri) => cx.open_url(&uri),
                    LinkTarget::File { path, line, column } => {
                        let target = crate::file_links::FileLinkTarget { path, line, column };
                        crate::file_links::open_file_link(target, cx);
                    }
                }
            }
            DragKind::Link { .. } => {}
            DragKind::Reported { .. } => {
                let modifiers = mouse_modifiers(event.modifiers);
                if interactive {
                    if let Some(bytes) = encode_mouse_release(
                        self.session.mode(),
                        button,
                        hit.viewport_column,
                        hit.viewport_row,
                        modifiers,
                        false,
                    ) {
                        let _ = self.session.write_user_bytes(&bytes);
                    }
                }
            }
            DragKind::Local {
                alt_click: true,
                moved: false,
                started_at,
                ..
            } if interactive
                && started_at.elapsed() <= Duration::from_millis(500)
                && self
                    .session
                    .selection_text()
                    .is_none_or(|selection| selection.chars().count() <= 1) =>
            {
                self.session.clear_selection();
                self.session
                    .move_cursor_to_viewport(hit.viewport_column, hit.viewport_row);
            }
            DragKind::Local { .. } => {
                if let Some(text) = self.session.selection_text().filter(|_| copy_on_select) {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
        }
    }

    fn spawn_autoscroll(&self, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            let keep_running = weak
                .update(cx, |this, _| this.autoscroll_tick(generation))
                .unwrap_or(false);
            if !keep_running {
                break;
            }
        })
        .detach();
    }

    fn autoscroll_tick(&mut self, generation: u64) -> bool {
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };
        if drag.generation != generation || !matches!(drag.kind, DragKind::Local { .. }) {
            return false;
        }
        if drag.autoscroll == 0 {
            return true;
        }

        self.session.scroll_local(drag.autoscroll);
        let display_offset = self.session.scroll_state().display_offset;
        let row = if drag.autoscroll > 0 {
            0
        } else {
            drag.rows.saturating_sub(1)
        };
        let point = viewport_to_point(
            display_offset,
            GridPoint::new(row, Column(drag.endpoint_column)),
        );
        self.session.update_selection(point, drag.endpoint_side);
        true
    }
}

fn mouse_button(button: GpuiMouseButton) -> Option<MouseButton> {
    match button {
        GpuiMouseButton::Left => Some(MouseButton::Left),
        GpuiMouseButton::Middle => Some(MouseButton::Middle),
        GpuiMouseButton::Right => Some(MouseButton::Right),
        GpuiMouseButton::Navigate(_) => None,
    }
}

fn mouse_modifiers(modifiers: gpui::Modifiers) -> MouseModifiers {
    MouseModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        control: modifiers.control,
    }
}

fn link_modifier(modifiers: gpui::Modifiers) -> bool {
    #[cfg(any(windows, target_os = "linux"))]
    {
        modifiers.control
    }
    #[cfg(target_os = "macos")]
    {
        modifiers.platform || modifiers.control
    }
}

fn autoscroll_amount(bounds: Bounds<Pixels>, position: Point<Pixels>) -> i32 {
    let distance = if position.y < bounds.top() {
        f32::from(bounds.top() - position.y)
    } else if position.y > bounds.bottom() {
        -f32::from(position.y - bounds.bottom())
    } else {
        return 0;
    };
    let normalized = distance.abs().min(50.) / 50.;
    let speed = 1 + (normalized * 14.).round() as i32;
    distance.signum() as i32 * speed
}

fn viewport_edge_hit(
    session: &TerminalMirror,
    geometry: TerminalGeometry,
    hit: TerminalHit,
    column_select: bool,
    top: bool,
) -> TerminalHit {
    let viewport_row = if top {
        0
    } else {
        geometry.rows.saturating_sub(1)
    };
    let viewport_column = if column_select {
        hit.viewport_column
    } else if top {
        0
    } else {
        geometry.cols.saturating_sub(1)
    };
    let side = if column_select {
        hit.side
    } else if top {
        Side::Left
    } else {
        Side::Right
    };
    let display_offset = session.scroll_state().display_offset;
    TerminalHit {
        point: point_for_viewport(display_offset, viewport_row, viewport_column),
        side,
        viewport_column,
        viewport_row,
    }
}

/// Adds a find match's cell to its row's quad. Cells join by grid column
/// and match: summed widths drift from `cell_width * col` at fractional
/// cell widths, so comparing pixel edges would split one match into boxes.
fn push_match_cell(
    quads: &mut Vec<Bounds<Pixels>>,
    end: &mut Option<(usize, usize, usize)>,
    (found, row, col, cell_cols): (usize, usize, usize, usize),
    cell: Bounds<Pixels>,
) {
    match (*end, quads.last_mut()) {
        (Some(last_end), Some(last)) if last_end == (found, row, col) => {
            last.size.width += cell.size.width;
        }
        _ => quads.push(cell),
    }
    *end = Some((found, row, col + cell_cols));
}

fn point_for_viewport(display_offset: usize, row: usize, column: usize) -> GridPoint {
    viewport_to_point(display_offset, GridPoint::new(row, Column(column)))
}

fn cell_and_side(local_x: f32, cell_width: f32, raw_column: usize) -> (usize, Side) {
    let offset = local_x - raw_column as f32 * cell_width;
    (
        raw_column,
        if offset <= cell_width / 2. {
            Side::Left
        } else {
            Side::Right
        },
    )
}

fn hit_test(
    session: &TerminalMirror,
    geometry: TerminalGeometry,
    position: Point<Pixels>,
) -> TerminalHit {
    let (raw_column, viewport_row) = viewport_cell(geometry, position);
    let local_x = f32::from(position.x - geometry.bounds.left()).clamp(
        0.,
        (f32::from(geometry.cell_width) * geometry.cols as f32).max(1.) - 0.001,
    );
    let term = session.term.lock_unfair();
    let display_offset = term.grid().display_offset();
    let (column, side) = cell_and_side(local_x, f32::from(geometry.cell_width), raw_column);
    TerminalHit {
        point: point_for_viewport(display_offset, viewport_row, column),
        side,
        viewport_column: raw_column,
        viewport_row,
    }
}

fn viewport_cell(geometry: TerminalGeometry, position: Point<Pixels>) -> (usize, usize) {
    let grid_width = f32::from(geometry.cell_width) * geometry.cols as f32;
    let grid_height = f32::from(geometry.line_height) * geometry.rows as f32;
    let local_x =
        f32::from(position.x - geometry.bounds.left()).clamp(0., grid_width.max(1.) - 0.001);
    let local_y =
        f32::from(position.y - geometry.bounds.top()).clamp(0., grid_height.max(1.) - 0.001);
    let raw_column = (local_x / f32::from(geometry.cell_width)) as usize;
    let viewport_row = (local_y / f32::from(geometry.line_height)) as usize;

    (raw_column, viewport_row)
}

fn accumulate_scroll(accumulator: &mut f32, lines: f32) -> i32 {
    *accumulator += lines;
    let whole = accumulator.trunc() as i32;
    *accumulator -= whole as f32;
    whole
}

/// A one-column cell cannot hold a color emoji, so a non-ASCII glyph is drawn
/// in text presentation unless U+FE0F asks for emoji (#1064).
fn wants_text_presentation(c: char, wide: bool, zerowidth: Option<&[char]>) -> bool {
    !wide && !c.is_ascii() && !zerowidth.is_some_and(|zw| zw.contains(&'\u{fe0f}'))
}

fn text_presentation_font(base: &Font) -> Font {
    let symbols = runner_app::platform_fonts::TERMINAL_SYMBOL_FALLBACKS;
    if symbols.is_empty() {
        return base.clone();
    }
    let mut families = base
        .fallbacks
        .as_ref()
        .map(|fallbacks| fallbacks.fallback_list().to_vec())
        .unwrap_or_default();
    families.extend(symbols.iter().map(|family| (*family).to_owned()));
    let mut font = base.clone();
    font.fallbacks = Some(FontFallbacks::from_fonts(families));
    font
}

pub(crate) fn to_hsla(rgb: Rgb, alpha: f32) -> Hsla {
    let mut rgba = gpui::rgb(((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32);
    rgba.a = alpha;
    rgba.into()
}

#[derive(Clone)]
pub struct TerminalElement {
    pub(super) session: Arc<TerminalMirror>,
    pub(super) interaction: Entity<TerminalInteraction>,
    pub(super) input: Entity<TerminalInput>,
    pub(super) focus_handle: FocusHandle,
    pub(super) interactive: bool,
    pub(super) scrollable: bool,
    pub(super) resize_owner: bool,
    pub(super) style: TerminalStyle,
    pub(super) find: Option<Entity<super::TerminalFind>>,
}

#[derive(Clone, PartialEq)]
pub struct TerminalStyle {
    pub palette: TerminalPalette,
    pub font: Font,
    pub font_size: f32,
    pub app_zoom: f32,
    pub line_height_factor: f32,
    pub copy_on_select: bool,
}

impl TerminalStyle {
    pub fn row_height(&self) -> f32 {
        row_height(self.font_size, self.line_height_factor)
    }
}

impl TerminalElement {
    pub fn new(
        session: Arc<TerminalMirror>,
        interaction: Entity<TerminalInteraction>,
        input: Entity<TerminalInput>,
        focus_handle: FocusHandle,
        interactive: bool,
        resize_owner: bool,
        style: TerminalStyle,
    ) -> Self {
        Self {
            session,
            interaction,
            input,
            focus_handle,
            interactive,
            scrollable: false,
            resize_owner,
            style,
            find: None,
        }
    }

    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    pub(crate) fn find(mut self, find: Entity<super::TerminalFind>) -> Self {
        self.find = Some(find);
        self
    }

    pub(crate) fn cached(self) -> super::viewport::TerminalViewportElement {
        super::viewport::TerminalViewportElement::new(self)
    }

    fn register_mouse_listeners(
        &self,
        geometry: TerminalGeometry,
        hitbox: Hitbox,
        window: &mut Window,
    ) {
        if self.scrollable {
            let interaction = self.interaction.clone();
            let interactive = self.interactive;
            let scroll_hitbox = hitbox.clone();
            let current_view = window.current_view();
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !scroll_hitbox.should_handle_scroll(window) {
                    return;
                }
                let changed = interaction.update(cx, |interaction, _| {
                    interaction.scroll_wheel(event, geometry, interactive, window)
                });
                if changed {
                    cx.notify(current_view);
                }
                cx.stop_propagation();
            });
        }
        let interaction = self.interaction.clone();
        let focus_handle = self.focus_handle.clone();
        let interactive = self.interactive;
        let down_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !down_hitbox.is_hovered(window) {
                return;
            }
            focus_handle.focus(window, cx);
            interaction.update(cx, |interaction, interaction_cx| {
                interaction.mouse_down(event, geometry, interactive, interaction_cx);
            });
        });

        let interaction = self.interaction.clone();
        let interactive = self.interactive;
        let move_hitbox = hitbox.clone();
        let current_view = window.current_view();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let changed = interaction.update(cx, |interaction, _| {
                interaction.refresh_hovered_link(
                    event.modifiers,
                    geometry,
                    event.position,
                    move_hitbox.is_hovered(window),
                    interactive,
                )
            });
            if changed {
                cx.notify(current_view);
                if interaction.read(cx).hovered_link.is_some() {
                    window
                        .spawn(cx, async move |cx| {
                            cx.background_executor().timer(LINK_TOOLTIP_DELAY).await;
                            cx.update(|window, _| window.refresh()).ok();
                        })
                        .detach();
                }
            }
            if event.pressed_button.is_none() {
                return;
            }
            if interaction.read(cx).drag.is_none() {
                return;
            }
            interaction.update(cx, |interaction, _| {
                interaction.mouse_move(event, geometry, interactive);
            });
        });

        let interaction = self.interaction.clone();
        let interactive = self.interactive;
        let copy_on_select = self.style.copy_on_select;
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || interaction.read(cx).drag.is_none() {
                return;
            }
            interaction.update(cx, |interaction, interaction_cx| {
                interaction.mouse_up(event, geometry, interactive, copy_on_select, interaction_cx);
            });
        });

        let interaction = self.interaction.clone();
        let modifier_hitbox = hitbox;
        let current_view = window.current_view();
        window.on_modifiers_changed(move |event, window, cx| {
            let hovered = modifier_hitbox.is_hovered(window);
            let changed = interaction.update(cx, |interaction, _| {
                interaction.refresh_hovered_link(
                    event.modifiers,
                    geometry,
                    window.mouse_position(),
                    hovered,
                    interactive,
                )
            });
            if hovered || changed {
                cx.notify(current_view);
            }
        });
    }
}

pub struct GridPrepaint {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    find_matches: Vec<Bounds<Pixels>>,
    find_current: Vec<Bounds<Pixels>>,
    selections: Vec<Bounds<Pixels>>,
    procedural: Vec<ProceduralCell>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, CursorShape)>,
    cursor_procedural: Option<ProceduralCell>,
    cursor_text: Option<(Point<Pixels>, ShapedLine)>,
    cursor_cell: Option<Bounds<Pixels>>,
    marked_text: Option<(Bounds<Pixels>, ShapedLine, Hsla)>,
    cell_width: Pixels,
    line_height: Pixels,
    cols: usize,
    rows: usize,
    mode: TermMode,
    hovered_link: Option<TerminalLink>,
    link_modifier_held: bool,
    hitbox: Hitbox,
}

struct StyledSpan {
    col: usize,
    text: String,
    runs: Vec<TextRun>,
    /// True only while every cell in the span is narrow ASCII; only
    /// such spans may be extended (their byte len == column count and
    /// the monospace advance is exact).
    ascii_only: bool,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = GridPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> GridPrepaint {
        #[cfg(test)]
        crate::render_counts::count_render(&format!("terminal:{}", self.session.session_id()), cx);
        let text_system = window.text_system();
        let base_font = self.style.font.clone();
        let symbol_font = text_presentation_font(&base_font);
        let font_size = px(self.style.font_size);
        let font_id = text_system.resolve_font(&base_font);
        let cell_width = text_system
            .em_advance(font_id, font_size)
            .unwrap_or(px(self.style.font_size * 0.6));
        let line_height = px(self.style.row_height());
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        if !self.session.control().desktop() {
            self.input.update(cx, |input, _| {
                input.cancel_composition();
            });
        }
        let measured = terminal_grid_size(
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            f32::from(cell_width),
            f32::from(line_height),
        );
        self.interaction
            .update(cx, |interaction, _| interaction.pane_size = measured);
        let (last_cols, last_rows) = self.session.size();
        match size_push_verdict(
            measured,
            TerminalGridSize {
                cols: last_cols,
                rows: last_rows,
            },
            self.resize_owner,
        ) {
            SizePushVerdict::Push(size) => {
                // Keep timing and formatting out of release prepaint unless
                // debug tracing is explicitly enabled.
                if tracing::enabled!(tracing::Level::DEBUG) {
                    let started = Instant::now();
                    self.session.resize(size.cols, size.rows);
                    tracing::debug!(
                        "terminal resize prepaint: session={} size={}x{} push_us={}",
                        self.session.session_id(),
                        size.cols,
                        size.rows,
                        started.elapsed().as_micros(),
                    );
                } else {
                    self.session.resize(size.cols, size.rows);
                }
            }
            SizePushVerdict::Unchanged | SizePushVerdict::SuppressedNonOwner => {}
            SizePushVerdict::SuppressedUnplaced => {
                return GridPrepaint {
                    backgrounds: Vec::new(),
                    find_matches: Vec::new(),
                    find_current: Vec::new(),
                    selections: Vec::new(),
                    procedural: Vec::new(),
                    lines: Vec::new(),
                    cursor: None,
                    cursor_procedural: None,
                    cursor_text: None,
                    cursor_cell: None,
                    marked_text: None,
                    cell_width,
                    line_height,
                    cols: last_cols as usize,
                    rows: last_rows as usize,
                    mode: TermMode::NONE,
                    hovered_link: None,
                    link_modifier_held: false,
                    hitbox,
                };
            }
        }
        let (cols, rows) = self.session.size();
        let geometry = TerminalGeometry {
            bounds,
            cell_width,
            line_height,
            cols: cols as usize,
            rows: rows as usize,
        };
        // Hitbox ids are fresh every frame and `mouse_hit_test` is only
        // recomputed after prepaint, so `hitbox.is_hovered` is always false
        // here; test the bounds directly, as gpui's own hover tooltips do.
        let hovered = bounds.contains(&window.mouse_position());
        let (hovered_link, link_modifier_held) = self.interaction.update(cx, |interaction, _| {
            interaction.refresh_hovered_link(
                window.modifiers(),
                geometry,
                window.mouse_position(),
                hovered,
                self.interactive,
            );
            (
                interaction.hovered_link.clone(),
                interaction.link_modifier_held,
            )
        });
        if let Some(link) = hovered_link
            .as_ref()
            .filter(|_| self.interaction.read(cx).link_tooltip_visible())
        {
            let mouse_position = window.mouse_position();
            let target = link.target.clone();
            let tooltip = self.interaction.update(cx, |interaction, cx| {
                interaction.link_tooltip(target, link_modifier_held, mouse_position, cx)
            });
            let interaction = self.interaction.downgrade();
            window.set_tooltip(AnyTooltip {
                view: tooltip.view,
                mouse_position: tooltip.mouse_position,
                check_visible_and_update: Rc::new(move |_, _, cx| {
                    interaction
                        .upgrade()
                        .is_some_and(|interaction| interaction.read(cx).link_tooltip_visible())
                }),
            });
        }

        let terminal_palette = self.style.palette;
        let base = palette::base_palette_for(terminal_palette);
        let mut backgrounds: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
        let mut selections: Vec<Bounds<Pixels>> = Vec::new();
        let mut selection_end = None;
        let mut find_matches: Vec<Bounds<Pixels>> = Vec::new();
        let mut find_current: Vec<Bounds<Pixels>> = Vec::new();
        let mut find_match_end = None;
        let mut find_current_end = None;
        let find_text = theme::find_highlight().text;
        let mut procedural = Vec::new();
        let mut spans: Vec<(usize, StyledSpan)> = Vec::new();
        let mut cursor = None;
        let mut cursor_procedural = None;
        let mut cursor_text = None;
        let mut cursor_cell = None;
        let mut cursor_cols = 1;
        let mut cursor_blinks = false;
        let mut cursor_point = GridPoint::default();
        let mode;
        let marked_foreground;
        let marked_background;

        {
            let term = self.session.term.lock();
            let content = term.renderable_content();
            mode = content.mode;
            let display_offset = content.display_offset;
            let overrides = content.colors;
            let selection = content.selection;
            let renderable_cursor = content.cursor;
            let highlights = self
                .find
                .as_ref()
                .and_then(|find| find.update(cx, |find, _| find.highlights(&term, rows as usize)));
            let (highlighted, current_match) = highlights
                .map(|highlights| (highlights.matches, highlights.current))
                .unwrap_or_default();
            let mut next_match = 0;
            marked_foreground = palette::resolve_for(
                Color::Named(NamedColor::Foreground),
                overrides,
                &base,
                terminal_palette,
            );
            marked_background = palette::resolve_for(
                Color::Named(NamedColor::Background),
                overrides,
                &base,
                terminal_palette,
            );

            for indexed in content.display_iter {
                let cell = &indexed.cell;
                let Some(vp) = point_to_viewport(display_offset, indexed.point) else {
                    continue;
                };
                let (row, col) = (vp.line, vp.column.0);
                if row >= rows as usize || col >= cols as usize {
                    continue;
                }

                let wide = cell.flags.contains(Flags::WIDE_CHAR);
                let cell_cols = if wide { 2usize } else { 1 };
                if indexed.point == renderable_cursor.point {
                    cursor_cols = cell_cols;
                }
                let selected = selection.is_some_and(|range| {
                    range.contains_cell(&indexed, renderable_cursor.point, CursorShape::Hidden)
                });
                if selected && !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    let quad_bounds = Bounds::new(
                        point(
                            bounds.left() + cell_width * col as f32,
                            bounds.top() + line_height * row as f32,
                        ),
                        size(cell_width * cell_cols as f32, line_height),
                    );
                    match (selection_end, selections.last_mut()) {
                        (Some((last_row, last_column)), Some(last))
                            if last_row == row && last_column == col =>
                        {
                            last.size.width += quad_bounds.size.width;
                        }
                        _ => selections.push(quad_bounds),
                    }
                    selection_end = Some((row, col + cell_cols));
                }

                while highlighted
                    .get(next_match)
                    .is_some_and(|found| *found.end() < indexed.point)
                {
                    next_match += 1;
                }
                let in_current = current_match
                    .as_ref()
                    .is_some_and(|found| found.contains(&indexed.point));
                let in_match = in_current
                    || highlighted
                        .get(next_match)
                        .is_some_and(|found| found.contains(&indexed.point));
                if in_match && !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    let quad_bounds = Bounds::new(
                        point(
                            bounds.left() + cell_width * col as f32,
                            bounds.top() + line_height * row as f32,
                        ),
                        size(cell_width * cell_cols as f32, line_height),
                    );
                    let cell_at = (next_match, row, col, cell_cols);
                    if in_current {
                        push_match_cell(
                            &mut find_current,
                            &mut find_current_end,
                            cell_at,
                            quad_bounds,
                        );
                    } else {
                        push_match_cell(
                            &mut find_matches,
                            &mut find_match_end,
                            cell_at,
                            quad_bounds,
                        );
                    }
                }

                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }

                let mut fg = palette::resolve_for(cell.fg, overrides, &base, terminal_palette);
                let mut bg = palette::resolve_for(cell.bg, overrides, &base, terminal_palette);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if cell.flags.contains(Flags::DIM) {
                    fg = fg * 0.66;
                }

                if bg != terminal_palette.background {
                    let quad_bounds = Bounds::new(
                        point(
                            bounds.left() + cell_width * col as f32,
                            bounds.top() + line_height * row as f32,
                        ),
                        size(cell_width * cell_cols as f32, line_height),
                    );
                    // Merge with the previous quad when contiguous and
                    // same color, to keep quad count sane.
                    match backgrounds.last_mut() {
                        Some((last, color))
                            if *color == to_hsla(bg, 1.)
                                && last.top() == quad_bounds.top()
                                && last.right() == quad_bounds.left() =>
                        {
                            last.size.width += quad_bounds.size.width;
                        }
                        _ => backgrounds.push((quad_bounds, to_hsla(bg, 1.))),
                    }
                }

                if cell.flags.contains(Flags::HIDDEN)
                    || (cell.c == ' ' && cell.zerowidth().is_none())
                {
                    continue;
                }

                let link_underlined = hovered_link
                    .as_ref()
                    .is_some_and(|link| link.contains(indexed.point));
                let uses_text_decorations = link_underlined
                    || cell
                        .flags
                        .intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
                if !uses_text_decorations {
                    if let Some(procedural_cell) = ProceduralCell::new(
                        cell.c,
                        snapped_cell_bounds(
                            bounds,
                            cell_width,
                            line_height,
                            row,
                            col,
                            window.scale_factor(),
                        ),
                        if in_current {
                            find_text
                        } else {
                            to_hsla(fg, 1.)
                        },
                        to_hsla(bg, 1.),
                        font_size,
                    ) {
                        if indexed.point == renderable_cursor.point
                            && renderable_cursor.shape == CursorShape::Block
                        {
                            cursor_procedural = ProceduralCell::new(
                                cell.c,
                                snapped_cell_bounds(
                                    bounds,
                                    cell_width,
                                    line_height,
                                    row,
                                    col,
                                    window.scale_factor(),
                                ),
                                to_hsla(terminal_palette.cursor_accent, 1.),
                                to_hsla(terminal_palette.cursor, 1.),
                                font_size,
                            );
                        }
                        procedural.push(procedural_cell);
                        continue;
                    }
                }

                let mut cell_font: Font = if wants_text_presentation(cell.c, wide, cell.zerowidth())
                {
                    symbol_font.clone()
                } else {
                    base_font.clone()
                };
                if cell.flags.contains(Flags::BOLD) || in_current {
                    cell_font = cell_font.bold();
                }
                if cell.flags.contains(Flags::ITALIC) {
                    cell_font = cell_font.italic();
                }
                let link_armed = link_underlined && link_modifier_held;
                let text_color = if in_current {
                    find_text
                } else if link_armed {
                    theme::accent()
                } else {
                    to_hsla(fg, 1.)
                };
                let underline = if link_underlined {
                    Some(gpui::UnderlineStyle {
                        color: Some(text_color),
                        thickness: px(self.style.app_zoom),
                        wavy: false,
                    })
                } else if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                    Some(gpui::UnderlineStyle {
                        color: Some(to_hsla(
                            cell.underline_color()
                                .map(|color| {
                                    palette::resolve_for(color, overrides, &base, terminal_palette)
                                })
                                .unwrap_or(fg),
                            1.,
                        )),
                        thickness: px(self.style.app_zoom),
                        wavy: cell.flags.contains(Flags::UNDERCURL),
                    })
                } else {
                    None
                };
                let strikethrough = if cell.flags.contains(Flags::STRIKEOUT) {
                    Some(gpui::StrikethroughStyle {
                        color: Some(to_hsla(fg, 1.)),
                        thickness: px(self.style.app_zoom),
                    })
                } else {
                    None
                };

                let mut text = String::new();
                text.push(cell.c);
                if let Some(zw) = cell.zerowidth() {
                    text.extend(zw.iter());
                }
                let run = TextRun {
                    len: text.len(),
                    font: cell_font,
                    color: text_color,
                    background_color: None,
                    underline,
                    strikethrough,
                };
                if indexed.point == renderable_cursor.point
                    && renderable_cursor.shape == CursorShape::Block
                {
                    let mut cursor_run = run.clone();
                    cursor_run.color = to_hsla(terminal_palette.cursor_accent, 1.);
                    cursor_run.underline = None;
                    cursor_run.strikethrough = None;
                    cursor_text = Some((
                        point(
                            bounds.left() + cell_width * col as f32,
                            bounds.top() + line_height * row as f32,
                        ),
                        window.text_system().shape_line(
                            SharedString::from(text.clone()),
                            font_size,
                            &[cursor_run],
                            None,
                        ),
                    ));
                }

                // Extend the open span only when BOTH the span so far
                // and this cell are simple ASCII — an ASCII cell after
                // a non-ASCII narrow glyph (box drawing, braille)
                // must start its own span, or it inherits the
                // fallback font's advance and skews off the grid.
                let simple = !wide && cell.c.is_ascii() && cell.zerowidth().is_none();
                let extended = simple
                    && match spans.last_mut() {
                        Some((last_row, span))
                            if *last_row == row
                                && span.ascii_only
                                && span.col + span.text.len() == col =>
                        {
                            match span.runs.last_mut() {
                                Some(last_run)
                                    if last_run.font == run.font
                                        && last_run.color == run.color
                                        && last_run.underline == run.underline
                                        && last_run.strikethrough == run.strikethrough =>
                                {
                                    last_run.len += run.len;
                                }
                                _ => span.runs.push(run.clone()),
                            }
                            span.text.push(cell.c);
                            true
                        }
                        _ => false,
                    };
                if !extended {
                    spans.push((
                        row,
                        StyledSpan {
                            col,
                            text,
                            runs: vec![run],
                            ascii_only: simple,
                        },
                    ));
                }
            }

            if let Some(vp) = point_to_viewport(
                display_offset,
                GridPoint::new(renderable_cursor.point.line, renderable_cursor.point.column),
            ) {
                if vp.line < rows as usize && vp.column.0 < cols as usize {
                    let origin = point(
                        bounds.left() + cell_width * vp.column.0 as f32,
                        bounds.top() + line_height * vp.line as f32,
                    );
                    let bounds =
                        Bounds::new(origin, size(cell_width * cursor_cols as f32, line_height));
                    cursor_cell = Some(bounds);
                    if renderable_cursor.shape != CursorShape::Hidden {
                        cursor = Some((bounds, renderable_cursor.shape));
                        cursor_blinks = term.cursor_style().blinking;
                        cursor_point = renderable_cursor.point;
                    }
                }
            }
        }
        let blinking = cursor_blinks
            && cursor.is_some()
            && self.focus_handle.is_focused(window)
            && window.is_window_active();
        let restart = (
            self.session.output_activity().last_seq,
            self.session.typed_count(),
            cursor_point,
        );
        if !self.interaction.update(cx, |interaction, cx| {
            interaction.cursor_visible(blinking, restart, cx)
        }) {
            cursor = None;
        }

        let lines = spans
            .into_iter()
            .map(|(row, span)| {
                let shaped = window.text_system().shape_line(
                    SharedString::from(span.text),
                    font_size,
                    &span.runs,
                    None,
                );
                (
                    point(
                        bounds.left() + cell_width * span.col as f32,
                        bounds.top() + line_height * row as f32,
                    ),
                    shaped,
                )
            })
            .collect();

        let marked_text =
            self.input
                .read(cx)
                .marked_text()
                .zip(cursor_cell)
                .map(|(text, cursor_bounds)| {
                    let color = to_hsla(marked_foreground, 1.);
                    let line = window.text_system().shape_line(
                        SharedString::from(text.to_owned()),
                        font_size,
                        &[TextRun {
                            len: text.len(),
                            font: base_font,
                            color,
                            background_color: None,
                            underline: Some(UnderlineStyle {
                                color: Some(color),
                                thickness: px(self.style.app_zoom),
                                wavy: false,
                            }),
                            strikethrough: None,
                        }],
                        None,
                    );
                    (cursor_bounds, line, to_hsla(marked_background, 1.))
                });

        GridPrepaint {
            backgrounds,
            find_matches,
            find_current,
            selections,
            procedural,
            lines,
            cursor,
            cursor_procedural,
            cursor_text,
            cursor_cell,
            marked_text,
            cell_width,
            line_height,
            cols: cols as usize,
            rows: rows as usize,
            mode,
            hovered_link,
            link_modifier_held,
            hitbox,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint: &mut GridPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_quad(fill(bounds, to_hsla(self.style.palette.background, 1.)));
        for (quad_bounds, color) in &prepaint.backgrounds {
            window.paint_quad(fill(*quad_bounds, *color));
        }
        let highlight = theme::find_highlight();
        let radius = px(2. * self.style.app_zoom);
        for quad_bounds in &prepaint.find_matches {
            window.paint_quad(fill(*quad_bounds, highlight.tint).corner_radii(radius));
        }
        for quad_bounds in &prepaint.find_current {
            window.paint_quad(fill(*quad_bounds, highlight.current).corner_radii(radius));
            window.paint_quad(
                outline(
                    quad_bounds.dilate(px(self.style.app_zoom)),
                    highlight.outline,
                    Default::default(),
                )
                .border_widths(px(self.style.app_zoom))
                .corner_radii(radius + px(self.style.app_zoom)),
            );
        }
        let selection_color = to_hsla(self.style.palette.selection, 1.);
        for quad_bounds in &prepaint.selections {
            window.paint_quad(fill(*quad_bounds, selection_color));
        }
        for cell in &prepaint.procedural {
            cell.paint(window);
        }
        for (origin, line) in &prepaint.lines {
            let _ = line.paint(
                *origin,
                prepaint.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        if let Some((cursor_bounds, line, background)) = &prepaint.marked_text {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.paint_quad(fill(
                    Bounds::new(
                        cursor_bounds.origin,
                        size(line.width.max(prepaint.cell_width), prepaint.line_height),
                    ),
                    *background,
                ));
                let _ = line.paint(
                    cursor_bounds.origin,
                    prepaint.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            });
        } else if let Some((cursor_bounds, shape)) = prepaint.cursor {
            let focused = self.focus_handle.is_focused(window);
            let color = to_hsla(self.style.palette.cursor, if focused { 1. } else { 0.5 });
            match shape {
                CursorShape::Block if focused => {
                    window.paint_quad(fill(cursor_bounds, color));
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: cursor_bounds,
                        }),
                        |window| {
                            if let Some(cell) = &prepaint.cursor_procedural {
                                cell.paint(window);
                            }
                            if let Some((origin, line)) = &prepaint.cursor_text {
                                let _ = line.paint(
                                    *origin,
                                    prepaint.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        },
                    );
                }
                CursorShape::Block | CursorShape::HollowBlock => {
                    window.paint_quad(outline(cursor_bounds, color, Default::default()));
                }
                CursorShape::Underline => {
                    let mut b = cursor_bounds;
                    b.origin.y = b.bottom() - px(2. * self.style.app_zoom);
                    b.size.height = px(2. * self.style.app_zoom);
                    window.paint_quad(fill(b, color));
                }
                CursorShape::Beam => {
                    let mut b = cursor_bounds;
                    b.size.width = px(2. * self.style.app_zoom);
                    window.paint_quad(fill(b, color));
                }
                CursorShape::Hidden => {}
            }
        }
        let input_bounds = prepaint.cursor_cell.unwrap_or_else(|| {
            Bounds::new(
                bounds.origin,
                size(prepaint.cell_width, prepaint.line_height),
            )
        });
        window.handle_input(
            &self.focus_handle,
            ElementInputHandler::new(input_bounds, self.input.clone()),
            cx,
        );
        let cursor_style = if prepaint.hovered_link.is_some() && prepaint.link_modifier_held {
            CursorStyle::PointingHand
        } else if window.modifiers().alt
            && !(self.interactive
                && prepaint.mode.intersects(TermMode::MOUSE_MODE)
                && !window.modifiers().shift)
        {
            CursorStyle::Crosshair
        } else if self.interactive && prepaint.mode.intersects(TermMode::MOUSE_MODE) {
            CursorStyle::Arrow
        } else {
            CursorStyle::IBeam
        };
        window.set_cursor_style(cursor_style, &prepaint.hitbox);
        self.register_mouse_listeners(
            TerminalGeometry {
                bounds,
                cell_width: prepaint.cell_width,
                line_height: prepaint.line_height,
                cols: prepaint.cols,
                rows: prepaint.rows,
            },
            prepaint.hitbox.clone(),
            window,
        );
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Condvar, Mutex, RwLock};

    use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
    use gpui::prelude::*;
    use gpui::{
        div, point, px, Bounds, Context, Entity, FocusHandle, Render, TestAppContext,
        VisualTestContext, Window,
    };
    use runner_daemon::session::runtime::{
        OutputStream, RuntimeOutput, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
        SpawnSpec,
    };
    use runner_daemon::{db, session, shell_path, AppCore};
    use runner_terminal::terminal::TerminalMirror;

    use super::{
        accumulate_scroll, autoscroll_amount, cell_and_side, link_modifier, point_for_viewport,
        push_match_cell, text_presentation_font, viewport_cell, wants_text_presentation,
        TerminalElement, TerminalGeometry, TerminalInput, TerminalInteraction, TerminalStyle,
    };
    use alacritty_terminal::term::TermMode;
    use runner_terminal::mappings::encode_scroll;

    #[derive(Default)]
    struct RecordingRuntime {
        outputs: Mutex<HashMap<String, std::sync::mpsc::Sender<RuntimeOutput>>>,
        writes: Mutex<Vec<(String, Vec<u8>)>>,
        written: Condvar,
    }

    impl RecordingRuntime {
        fn wait_for_writes(&self, count: usize) {
            let (writes, timeout) = self
                .written
                .wait_timeout_while(
                    self.writes.lock().unwrap(),
                    std::time::Duration::from_secs(5),
                    |writes| writes.len() < count,
                )
                .unwrap();
            assert!(!timeout.timed_out(), "queued terminal input did not arrive");
            assert_eq!(writes.len(), count);
        }
    }

    impl SessionRuntime for RecordingRuntime {
        fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
            let (tx, rx) = std::sync::mpsc::channel();
            self.outputs
                .lock()
                .unwrap()
                .insert(spec.session_id.clone(), tx);
            Ok((
                RuntimeSession {
                    runtime: "recording".into(),
                    session_id: spec.session_id,
                },
                OutputStream::new(rx, Arc::new(AtomicBool::new(false))),
            ))
        }

        fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
            self.outputs.lock().unwrap().remove(&session.session_id);
            Ok(())
        }

        fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
            self.writes
                .lock()
                .unwrap()
                .push((session.session_id.clone(), bytes.to_vec()));
            self.written.notify_all();
            Ok(())
        }

        fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()> {
            self.send_bytes(session, key.as_bytes())
        }

        fn resize(&self, _: &RuntimeSession, _: u16, _: u16) -> RuntimeResult<()> {
            Ok(())
        }

        fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
            Ok(Some(SessionStatus {
                alive: true,
                ..Default::default()
            }))
        }
    }

    fn test_core(root: &std::path::Path, runtime: Arc<RecordingRuntime>) -> AppCore {
        let app_data_dir = root.join("app-data");
        std::fs::create_dir_all(&app_data_dir).unwrap();
        let pool = Arc::new(db::open_fixture_pool(&app_data_dir.join("runner.db")).unwrap());
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        crate::test_support::core(
            pool,
            app_data_dir,
            session::SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                runtime,
            ),
            runtime_shell_env,
            runtime_discovery,
        )
    }

    fn spawn_terminal(
        core: &AppCore,
        runtime: &RecordingRuntime,
        role: &runner_daemon::model::Role,
        root: &std::path::Path,
    ) -> Arc<TerminalMirror> {
        let spawned = core
            .sessions
            .spawn_direct(
                role,
                None,
                None,
                None,
                None,
                Some(root.to_str().unwrap()),
                Some(80),
                Some(24),
                &core.app_data_dir,
                Arc::clone(&core.db),
                Arc::new(core.session_events()),
                None,
            )
            .unwrap();
        let terminal = TerminalMirror::attach(
            crate::test_support::client(core),
            spawned.id.clone(),
            Arc::new(|| {}),
        )
        .unwrap();
        emit_output(runtime, &terminal, b"\x1b[?1000h\x1b[?1006h", |terminal| {
            terminal.mode().contains(TermMode::SGR_MOUSE)
        });
        terminal
    }

    // Output goes through the host so the mirror keeps the host's frame
    // sequence and accepts its resize acknowledgments.
    fn emit_output(
        runtime: &RecordingRuntime,
        terminal: &TerminalMirror,
        bytes: &[u8],
        arrived: impl Fn(&TerminalMirror) -> bool,
    ) {
        runtime.outputs.lock().unwrap()[terminal.session_id()]
            .send(RuntimeOutput::Stream(bytes.to_vec()))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !arrived(terminal) {
            assert!(
                std::time::Instant::now() < deadline,
                "host output did not reach the mirror"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn wait_for_rows(terminal: &TerminalMirror, rows: u16) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while terminal.size().1 != rows {
            assert!(
                std::time::Instant::now() < deadline,
                "terminal resize was not acknowledged: {:?}",
                terminal.size()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[derive(Clone)]
    struct WheelPane {
        terminal: Arc<TerminalMirror>,
        interaction: Entity<TerminalInteraction>,
        input: Entity<TerminalInput>,
        focus: FocusHandle,
    }

    struct WheelHost {
        left: WheelPane,
        right: WheelPane,
        left_interactive: bool,
        right_interactive: bool,
        right_scrollable: bool,
        font_size: f32,
        gap: f32,
        line_height_factor: f32,
        copy_on_select: bool,
    }

    impl Render for WheelHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let style = TerminalStyle {
                palette: runner_terminal::palette::RUNNER,
                font: crate::app_settings::AppSettings::default()
                    .terminal_font_family
                    .font(cx),
                font_size: self.font_size,
                app_zoom: 1.,
                line_height_factor: self.line_height_factor,
                copy_on_select: self.copy_on_select,
            };
            div()
                .size_full()
                .flex()
                .p(px(20.))
                .gap(px(self.gap))
                .child(
                    div()
                        .debug_selector(|| "WHEEL_LEFT".into())
                        .w(px(260.))
                        .h(px(180.))
                        .p(px(10.))
                        .child(
                            TerminalElement::new(
                                Arc::clone(&self.left.terminal),
                                self.left.interaction.clone(),
                                self.left.input.clone(),
                                self.left.focus.clone(),
                                self.left_interactive,
                                true,
                                style.clone(),
                            )
                            .scrollable(true),
                        ),
                )
                .child(
                    div()
                        .debug_selector(|| "WHEEL_RIGHT".into())
                        .w(px(260.))
                        .h(px(180.))
                        .p(px(10.))
                        .child(
                            TerminalElement::new(
                                Arc::clone(&self.right.terminal),
                                self.right.interaction.clone(),
                                self.right.input.clone(),
                                self.right.focus.clone(),
                                self.right_interactive,
                                true,
                                style,
                            )
                            .scrollable(self.right_scrollable),
                        ),
                )
        }
    }

    #[test]
    fn a_find_match_is_one_box_per_row_at_a_fractional_cell_width() {
        use gpui::size;
        let (left, cell_width, line_height) = (px(6.), px(10.4), px(20.8));
        let mut quads = Vec::new();
        let mut end = None;
        let mut push = |found, row: usize, cols: std::ops::Range<usize>| {
            for col in cols {
                let cell = Bounds::new(
                    point(left + cell_width * col as f32, line_height * row as f32),
                    size(cell_width, line_height),
                );
                push_match_cell(&mut quads, &mut end, (found, row, col, 1), cell);
            }
        };
        push(0, 1, 5..14);
        push(1, 1, 14..16);
        push(1, 2, 0..3);
        let spans = quads
            .iter()
            .map(|quad| {
                (
                    ((quad.left() - left) / cell_width).round() as usize,
                    (quad.size.width / cell_width).round() as usize,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(spans, [(5, 9), (14, 2), (0, 3)]);
    }

    #[test]
    fn rendered_wheels_target_one_pane_and_follow_its_current_geometry() {
        use gpui::{size, ScrollDelta, ScrollWheelEvent};

        let temp = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RecordingRuntime::default());
        let core = test_core(temp.path(), Arc::clone(&runtime));
        let role = runner_daemon::ops::role::create(
            &core.db.get().unwrap(),
            runner_daemon::ops::role::CreateRoleInput {
                handle: "wheel-probe".into(),
                display_name: "Wheel probe".into(),
                runtime: runner_daemon::model::Runtime::Trae,
                command: "probe".into(),
                args: Vec::new(),
                working_dir: None,
                system_prompt: None,
                env: Default::default(),
                model: None,
                effort: None,
                codex_speed: None,
                permission_mode: runner_daemon::router::runtime::PermissionMode::Auto,
            },
        )
        .unwrap();
        let left_terminal = spawn_terminal(&core, &runtime, &role, temp.path());
        let right_terminal = spawn_terminal(&core, &runtime, &role, temp.path());
        for terminal in [&left_terminal, &right_terminal] {
            emit_output(
                &runtime,
                terminal,
                &"scrollback line\r\n".repeat(40).into_bytes(),
                |terminal| terminal.scroll_state().history_lines > 0,
            );
        }
        let right_id = right_terminal.session_id().to_owned();
        let mut cx = TestAppContext::single();
        let left = WheelPane {
            interaction: cx.new(|_| TerminalInteraction::new(Arc::clone(&left_terminal))),
            input: cx.new(|_| TerminalInput::new(Arc::clone(&left_terminal))),
            focus: cx.update(|cx| cx.focus_handle()),
            terminal: Arc::clone(&left_terminal),
        };
        let right = WheelPane {
            interaction: cx.new(|_| TerminalInteraction::new(Arc::clone(&right_terminal))),
            input: cx.new(|_| TerminalInput::new(Arc::clone(&right_terminal))),
            focus: cx.update(|cx| cx.focus_handle()),
            terminal: Arc::clone(&right_terminal),
        };
        let window = cx.add_window(|_, _| WheelHost {
            left,
            right,
            left_interactive: true,
            right_interactive: true,
            right_scrollable: true,
            font_size: 14.,
            gap: 20.,
            line_height_factor: crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT,
            copy_on_select: false,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(size(px(700.), px(300.)));
        let right_bounds = visual.debug_bounds("WHEEL_RIGHT").unwrap();
        let cell_width = visual.update(|window, cx| {
            let font = crate::app_settings::AppSettings::default()
                .terminal_font_family
                .font(cx);
            let id = window.text_system().resolve_font(&font);
            window
                .text_system()
                .em_advance(id, px(14.))
                .unwrap_or(px(8.4))
        });
        let body = point(
            right_bounds.left() + px(10.) + cell_width * 7. + px(1.),
            right_bounds.top()
                + px(10.)
                + px(super::row_height(
                    14.,
                    crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT,
                )) * 4.
                + px(1.),
        );
        let header = point(body.x, right_bounds.top() + px(11.));
        let wheel = |position, delta| ScrollWheelEvent {
            position,
            delta: ScrollDelta::Lines(point(0., delta)),
            ..Default::default()
        };
        visual.simulate_event(wheel(body, 1.));
        runtime.wait_for_writes(1);
        assert_eq!(
            runtime.writes.lock().unwrap().as_slice(),
            &[(right_id.clone(), b"\x1b[<64;8;5M".to_vec())]
        );
        visual.simulate_event(wheel(header, 1.));
        runtime.wait_for_writes(2);
        assert_eq!(
            runtime.writes.lock().unwrap().last(),
            Some(&(right_id.clone(), b"\x1b[<64;8;1M".to_vec()))
        );
        assert_eq!(runtime.writes.lock().unwrap().len(), 2);
        visual.simulate_event(wheel(body, 0.5));
        assert_eq!(runtime.writes.lock().unwrap().len(), 2);
        let half_line = visual.update(|window, _| window.line_height() / 2.);
        visual.simulate_event(ScrollWheelEvent {
            position: body,
            delta: ScrollDelta::Pixels(point(px(0.), half_line)),
            ..Default::default()
        });
        runtime.wait_for_writes(3);
        assert_eq!(
            runtime.writes.lock().unwrap().last(),
            Some(&(right_id.clone(), b"\x1b[<64;8;5M".to_vec()))
        );
        assert_eq!(runtime.writes.lock().unwrap().len(), 3);

        window
            .update(&mut visual, |host, _, cx| {
                host.font_size = 20.;
                host.gap = 40.;
                cx.notify();
            })
            .unwrap();
        visual.refresh().unwrap();
        // A late shrink acknowledgment would add the lost rows to a nonzero display offset.
        wait_for_rows(&left_terminal, 5);
        wait_for_rows(&right_terminal, 5);
        let changed_bounds = visual.debug_bounds("WHEEL_RIGHT").unwrap();
        assert_eq!(changed_bounds.left(), right_bounds.left() + px(20.));
        let new_width = visual.update(|window, cx| {
            let font = crate::app_settings::AppSettings::default()
                .terminal_font_family
                .font(cx);
            let id = window.text_system().resolve_font(&font);
            window
                .text_system()
                .em_advance(id, px(20.))
                .unwrap_or(px(12.))
        });
        let changed_body = point(
            changed_bounds.left() + px(10.) + new_width * 5. + px(1.),
            changed_bounds.top()
                + px(10.)
                + px(super::row_height(
                    20.,
                    crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT,
                )) * 3.
                + px(1.),
        );
        visual.simulate_event(wheel(changed_body, 1.));
        runtime.wait_for_writes(4);
        assert_eq!(
            runtime.writes.lock().unwrap().last(),
            Some(&(right_id.clone(), b"\x1b[<64;6;4M".to_vec()))
        );
        visual.simulate_event(ScrollWheelEvent {
            position: changed_body,
            delta: ScrollDelta::Lines(point(0., 1.)),
            modifiers: gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            ..Default::default()
        });
        assert_eq!(runtime.writes.lock().unwrap().len(), 4);
        assert_eq!(right_terminal.scroll_state().display_offset, 1);

        window
            .update(&mut visual, |host, _, cx| {
                host.right_scrollable = false;
                host.left_interactive = false;
                cx.notify();
            })
            .unwrap();
        visual.refresh().unwrap();
        visual.simulate_event(wheel(changed_body, 1.));
        assert_eq!(runtime.writes.lock().unwrap().len(), 4);
        let left_bounds = visual.debug_bounds("WHEEL_LEFT").unwrap();
        visual.simulate_event(wheel(
            point(left_bounds.left() + px(20.), left_bounds.top() + px(20.)),
            1.,
        ));
        assert_eq!(runtime.writes.lock().unwrap().len(), 4);
        assert_eq!(left_terminal.scroll_state().display_offset, 1);
    }

    fn probe_role(core: &AppCore) -> runner_daemon::model::Role {
        runner_daemon::ops::role::create(
            &core.db.get().unwrap(),
            runner_daemon::ops::role::CreateRoleInput {
                handle: "settings-probe".into(),
                display_name: "Settings probe".into(),
                runtime: runner_daemon::model::Runtime::Trae,
                command: "probe".into(),
                args: Vec::new(),
                working_dir: None,
                system_prompt: None,
                env: Default::default(),
                model: None,
                effort: None,
                codex_speed: None,
                permission_mode: runner_daemon::router::runtime::PermissionMode::Auto,
            },
        )
        .unwrap()
    }

    fn wheel_pane(cx: &mut TestAppContext, terminal: &Arc<TerminalMirror>) -> WheelPane {
        WheelPane {
            interaction: cx.new(|_| TerminalInteraction::new(Arc::clone(terminal))),
            input: cx.new(|_| TerminalInput::new(Arc::clone(terminal))),
            focus: cx.update(|cx| cx.focus_handle()),
            terminal: Arc::clone(terminal),
        }
    }

    #[test]
    fn row_height_and_pty_rows_follow_the_line_height_setting() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RecordingRuntime::default());
        let core = test_core(temp.path(), Arc::clone(&runtime));
        let role = probe_role(&core);
        let left_terminal = spawn_terminal(&core, &runtime, &role, temp.path());
        let right_terminal = spawn_terminal(&core, &runtime, &role, temp.path());
        let mut cx = TestAppContext::single();
        let (left, right) = (
            wheel_pane(&mut cx, &left_terminal),
            wheel_pane(&mut cx, &right_terminal),
        );
        let window = cx.add_window(|_, _| WheelHost {
            left,
            right,
            left_interactive: true,
            right_interactive: true,
            right_scrollable: true,
            font_size: 14.,
            gap: 20.,
            line_height_factor: 1.0,
            copy_on_select: false,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(700.), px(300.)));
        let cell_width = visual.update(|window, cx| {
            let font = crate::app_settings::AppSettings::default()
                .terminal_font_family
                .font(cx);
            let id = window.text_system().resolve_font(&font);
            f32::from(window.text_system().em_advance(id, px(14.)).unwrap())
        });
        let expected_rows = |factor: f32| {
            runner_app::terminal_resize::terminal_grid_size(
                240.,
                160.,
                cell_width,
                super::row_height(14., factor),
            )
            .unwrap()
            .rows
        };
        assert_eq!(super::row_height(14., 1.0), 14.);
        assert_eq!(super::row_height(14., 1.6), 22.);
        let mut seen = Vec::new();
        for factor in [1.0, 1.6, crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT] {
            window
                .update(&mut visual, |host, _, cx| {
                    host.line_height_factor = factor;
                    cx.notify();
                })
                .unwrap();
            visual.refresh().unwrap();
            let rows = expected_rows(factor);
            wait_for_rows(&left_terminal, rows);
            wait_for_rows(&right_terminal, rows);
            seen.push(rows);
        }
        assert!(seen[0] > seen[2] && seen[2] > seen[1], "{seen:?}");
    }

    #[test]
    fn copy_on_select_copies_a_finished_drag_only_when_on() {
        use gpui::{Modifiers, MouseButton};

        let temp = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RecordingRuntime::default());
        let core = test_core(temp.path(), Arc::clone(&runtime));
        let role = probe_role(&core);
        let left_terminal = spawn_terminal(&core, &runtime, &role, temp.path());
        let right_terminal = spawn_terminal(&core, &runtime, &role, temp.path());
        emit_output(
            &runtime,
            &left_terminal,
            b"\x1b[?1000l\x1b[?1006lcopy me please\r\n",
            |terminal| !terminal.mode().contains(TermMode::SGR_MOUSE),
        );
        let mut cx = TestAppContext::single();
        let (left, right) = (
            wheel_pane(&mut cx, &left_terminal),
            wheel_pane(&mut cx, &right_terminal),
        );
        let window = cx.add_window(|_, _| WheelHost {
            left,
            right,
            left_interactive: true,
            right_interactive: true,
            right_scrollable: true,
            font_size: 14.,
            gap: 20.,
            line_height_factor: crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT,
            copy_on_select: false,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(700.), px(300.)));
        let bounds = visual.debug_bounds("WHEEL_LEFT").unwrap();
        let row = bounds.top() + px(10. + super::row_height(14., 1.4) / 2.);
        let start = point(bounds.left() + px(11.), row);
        let end = point(bounds.left() + px(60.), row);
        let drag = |visual: &mut VisualTestContext| {
            visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
            visual.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
            visual.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
            visual.run_until_parked();
        };
        let clipboard = |visual: &mut VisualTestContext| {
            visual.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
        };

        visual
            .update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("kept".into())));
        drag(&mut visual);
        let selected = left_terminal.selection_text().unwrap();
        assert!(selected.starts_with("copy"), "{selected:?}");
        assert_eq!(clipboard(&mut visual).as_deref(), Some("kept"));

        window
            .update(&mut visual, |host, _, cx| {
                host.copy_on_select = true;
                cx.notify();
            })
            .unwrap();
        visual.refresh().unwrap();
        drag(&mut visual);
        assert_eq!(clipboard(&mut visual), Some(selected.clone()));
        assert_eq!(left_terminal.selection_text(), Some(selected));
    }

    #[test]
    fn blinking_cursor_toggles_on_one_timer_and_shows_again_on_output_or_input() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RecordingRuntime::default());
        let core = test_core(temp.path(), Arc::clone(&runtime));
        let terminal = spawn_terminal(&core, &runtime, &probe_role(&core), temp.path());
        let typed = terminal.typed_count();
        terminal
            .send_key("a", false, false, false, Some("a"))
            .unwrap();
        terminal.send_text("b").unwrap();
        terminal.paste("c").unwrap();
        assert_eq!(
            terminal.typed_count(),
            typed + 3,
            "keys, text and pastes count"
        );
        let mut cx = TestAppContext::single();
        let interaction = cx.new(|_| TerminalInteraction::new(terminal));
        let at = GridPoint::new(Line(0), Column(0));
        let visible = |cx: &mut TestAppContext, blinking: bool, restart: (u64, u64, GridPoint)| {
            interaction.update(cx, |interaction, cx| {
                interaction.cursor_visible(blinking, restart, cx)
            })
        };
        let wait = |cx: &mut TestAppContext, duration: std::time::Duration| {
            cx.executor().advance_clock(duration);
            cx.run_until_parked();
        };
        let interval = super::CURSOR_BLINK_INTERVAL;
        let half = interval / 2;
        let timer = |cx: &mut TestAppContext| {
            interaction.read_with(cx, |interaction, _| {
                (interaction.blink.running, interaction.blink.task.is_some())
            })
        };

        assert!(visible(&mut cx, false, (1, 0, at)));
        assert_eq!(timer(&mut cx), (false, false));

        assert!(visible(&mut cx, true, (1, 0, at)));
        wait(&mut cx, interval);
        assert!(!visible(&mut cx, true, (1, 0, at)));
        wait(&mut cx, interval);
        assert!(visible(&mut cx, true, (1, 0, at)));
        wait(&mut cx, interval);
        assert!(!visible(&mut cx, true, (1, 0, at)));

        assert!(
            visible(&mut cx, true, (2, 0, at)),
            "output shows the cursor"
        );
        wait(&mut cx, interval);
        assert!(!visible(&mut cx, true, (2, 0, at)));
        assert!(
            visible(&mut cx, true, (2, 1, at)),
            "typing shows the cursor"
        );
        wait(&mut cx, interval);
        assert!(!visible(&mut cx, true, (2, 1, at)));

        // Restarts inside one interval push the next toggle back.
        assert!(visible(&mut cx, true, (3, 1, at)));
        for seq in 4..8 {
            wait(&mut cx, half);
            assert!(visible(&mut cx, true, (seq, 1, at)), "restart {seq}");
        }
        wait(&mut cx, half);
        assert!(visible(&mut cx, true, (7, 1, at)));
        wait(&mut cx, half);
        assert!(!visible(&mut cx, true, (7, 1, at)));

        assert!(
            visible(&mut cx, false, (7, 1, at)),
            "unfocused draws it steadily"
        );
        assert_eq!(
            timer(&mut cx),
            (false, false),
            "turning blink off cancels the timer"
        );

        assert!(visible(&mut cx, true, (7, 1, at)));
        assert_eq!(timer(&mut cx), (true, true));
        wait(&mut cx, interval);
        wait(&mut cx, interval);
        assert!(
            !timer(&mut cx).0,
            "a terminal that stops painting stops its timer"
        );
    }

    #[test]
    fn wheel_pointer_uses_displayed_pane_geometry_through_encoding() {
        let geometry = TerminalGeometry {
            bounds: Bounds::new(point(px(120.), px(80.)), gpui::size(px(200.), px(160.))),
            cell_width: px(10.),
            line_height: px(20.),
            cols: 20,
            rows: 8,
        };
        let header = viewport_cell(geometry, point(px(195.), px(85.)));
        let content = viewport_cell(geometry, point(px(195.), px(165.)));
        assert_eq!(header, (7, 0));
        assert_eq!(content, (7, 4));
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            encode_scroll(mode, 1, false, content.0, content.1),
            Some(b"\x1b[<64;8;5M".to_vec())
        );
        assert_eq!(
            encode_scroll(mode, 1, false, header.0, header.1),
            Some(b"\x1b[<64;8;1M".to_vec())
        );

        let changed = TerminalGeometry {
            cell_width: px(15.),
            line_height: px(25.),
            cols: 13,
            rows: 6,
            ..geometry
        };
        assert_eq!(viewport_cell(changed, point(px(195.), px(165.))), (5, 3));
        assert_eq!(viewport_cell(changed, point(px(119.), px(79.))), (0, 0));
        assert_eq!(viewport_cell(changed, point(px(500.), px(500.))), (12, 5));
    }

    #[test]
    fn narrow_symbols_use_text_presentation_unless_emoji_is_requested() {
        assert!(wants_text_presentation('\u{23fa}', false, None));
        assert!(wants_text_presentation(
            '\u{23f8}',
            false,
            Some(&['\u{fe0e}'])
        ));
        assert!(!wants_text_presentation(
            '\u{23fa}',
            false,
            Some(&['\u{fe0f}'])
        ));
        assert!(!wants_text_presentation('\u{1f680}', true, None));
        assert!(!wants_text_presentation('a', false, None));
    }

    #[test]
    fn text_presentation_font_adds_the_platform_symbol_fallbacks() {
        let mut base = gpui::font("JetBrainsMono Nerd Font Mono");
        base.fallbacks = Some(gpui::FontFallbacks::from_fonts(vec![
            "Microsoft YaHei".to_owned(),
            "Segoe UI".to_owned(),
        ]));
        let font = text_presentation_font(&base);
        #[cfg(windows)]
        {
            assert_eq!(font.family, base.family);
            assert_eq!(
                font.fallbacks.unwrap().fallback_list(),
                ["Microsoft YaHei", "Segoe UI", "Segoe UI Symbol"]
            );
        }
        #[cfg(not(windows))]
        assert_eq!(font, base);
    }

    #[test]
    fn fractional_wheel_deltas_emit_only_accumulated_whole_lines() {
        let mut accumulator = 0.;
        assert_eq!(accumulate_scroll(&mut accumulator, 0.6), 0);
        assert_eq!(accumulate_scroll(&mut accumulator, 0.6), 1);
        assert!((accumulator - 0.2).abs() < 0.001);
        assert_eq!(accumulate_scroll(&mut accumulator, -0.6), 0);
        assert_eq!(accumulate_scroll(&mut accumulator, -0.6), -1);
    }

    #[test]
    fn terminal_links_use_control_on_windows_and_preserve_macos_modifiers() {
        assert!(!link_modifier(gpui::Modifiers::default()));
        assert!(!link_modifier(gpui::Modifiers {
            alt: true,
            ..Default::default()
        }));
        assert_eq!(
            link_modifier(gpui::Modifiers {
                platform: true,
                ..Default::default()
            }),
            cfg!(target_os = "macos"),
        );
        assert!(link_modifier(gpui::Modifiers {
            control: true,
            ..Default::default()
        }));
    }

    #[test]
    fn pixel_hit_testing_uses_half_cells_and_exact_edges() {
        assert_eq!(cell_and_side(0., 10., 0), (0, Side::Left));
        assert_eq!(cell_and_side(4.999, 10., 0), (0, Side::Left));
        assert_eq!(cell_and_side(5., 10., 0), (0, Side::Left));
        assert_eq!(cell_and_side(5.001, 10., 0), (0, Side::Right));
        assert_eq!(cell_and_side(20., 10., 2), (2, Side::Left));
    }

    #[test]
    fn wide_cell_hit_testing_keeps_primary_and_spacer_cell_boundaries() {
        assert_eq!(cell_and_side(30., 10., 3), (3, Side::Left));
        assert_eq!(cell_and_side(35., 10., 3), (3, Side::Left));
        assert_eq!(cell_and_side(35.001, 10., 3), (3, Side::Right));
        assert_eq!(cell_and_side(40., 10., 4), (4, Side::Left));
        assert_eq!(cell_and_side(45., 10., 4), (4, Side::Left));
        assert_eq!(cell_and_side(45.001, 10., 4), (4, Side::Right));
    }

    #[test]
    fn viewport_rows_include_the_scrollback_offset() {
        assert_eq!(
            point_for_viewport(7, 0, 4),
            GridPoint::new(Line(-7), Column(4))
        );
        assert_eq!(
            point_for_viewport(7, 6, 4),
            GridPoint::new(Line(-1), Column(4))
        );
    }

    #[test]
    fn drag_distance_controls_autoscroll_direction_and_speed() {
        let bounds = Bounds::new(point(px(10.), px(20.)), gpui::size(px(100.), px(80.)));
        assert_eq!(autoscroll_amount(bounds, point(px(20.), px(40.))), 0);
        assert_eq!(autoscroll_amount(bounds, point(px(20.), px(19.))), 1);
        assert_eq!(autoscroll_amount(bounds, point(px(20.), px(110.))), -4);
        assert_eq!(autoscroll_amount(bounds, point(px(20.), px(220.))), -15);
        assert_eq!(autoscroll_amount(bounds, point(px(20.), px(500.))), -15);
    }
}
