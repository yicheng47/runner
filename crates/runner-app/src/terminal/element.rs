//! Custom GPUI element painting the `alacritty_terminal` grid: cell
//! backgrounds as quads, glyphs as shaped lines, cursor on top.
//!
//! Alignment strategy: same-style ASCII spans are shaped as one run
//! (monospace advance == cell width, so columns stay true); custom
//! terminal graphics are painted procedurally; remaining non-ASCII is
//! shaped per cell and painted at its own column origin, so a fallback
//! font's advance can never skew the grid. gpui caches shaped lines,
//! so per-cell shaping of repetitive glyphs stays cheap.

use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Point as GridPoint, Side};
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{point_to_viewport, viewport_to_point, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui::{
    fill, font, outline, point, px, relative, size, App, Bounds, ContentMask, Context, CursorStyle,
    DispatchPhase, Element, ElementInputHandler, Entity, FocusHandle, Font, GlobalElementId,
    Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton as GpuiMouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ShapedLine, SharedString, Style, TextRun, UnderlineStyle, Window,
};

use runner_app::terminal_ime::TerminalInput;
use runner_app::terminal_resize::{
    size_push_verdict, terminal_grid_size, SizePushVerdict, TerminalGridSize,
};
use runner_terminal::mappings::{
    encode_mouse_motion, encode_mouse_press, encode_mouse_release, MouseButton, MouseModifiers,
};
use runner_terminal::palette::{self, TerminalPalette};
use runner_terminal::terminal::{TerminalLink, TerminalSession};

use super::glyphs::{snapped_cell_bounds, ProceduralCell};

pub const LINE_HEIGHT_FACTOR: f32 = 1.4;

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
        uri: String,
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

pub(crate) struct TerminalInteraction {
    session: Arc<TerminalSession>,
    drag: Option<DragState>,
    next_generation: u64,
    hovered_link: Option<TerminalLink>,
    hover_signature: Option<(GridPoint, u64, usize, (u16, u16))>,
    hover_modifiers: Option<(bool, bool, bool, bool)>,
}

impl TerminalInteraction {
    pub(crate) fn new(session: Arc<TerminalSession>) -> Self {
        Self {
            session,
            drag: None,
            next_generation: 0,
            hovered_link: None,
            hover_signature: None,
            hover_modifiers: None,
        }
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
        if !interactive || !link_modifier(modifiers) {
            self.hover_signature = None;
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
                        uri: link.uri,
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
                uri, moved: false, ..
            } if interactive
                && link_modifier(event.modifiers)
                && self
                    .session
                    .link_at(hit.point)
                    .is_some_and(|link| link.uri == uri) =>
            {
                cx.open_url(&uri);
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
            DragKind::Local { .. } => {}
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

        self.session.scroll(drag.autoscroll, true);
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
    modifiers.platform || modifiers.control
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
    session: &TerminalSession,
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
    session: &TerminalSession,
    geometry: TerminalGeometry,
    position: Point<Pixels>,
) -> TerminalHit {
    let grid_width = f32::from(geometry.cell_width) * geometry.cols as f32;
    let grid_height = f32::from(geometry.line_height) * geometry.rows as f32;
    let local_x =
        f32::from(position.x - geometry.bounds.left()).clamp(0., grid_width.max(1.) - 0.001);
    let local_y =
        f32::from(position.y - geometry.bounds.top()).clamp(0., grid_height.max(1.) - 0.001);
    let raw_column = (local_x / f32::from(geometry.cell_width)) as usize;
    let viewport_row = (local_y / f32::from(geometry.line_height)) as usize;

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

pub(crate) fn to_hsla(rgb: Rgb, alpha: f32) -> Hsla {
    let mut rgba = gpui::rgb(((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32);
    rgba.a = alpha;
    rgba.into()
}

pub struct TerminalElement {
    session: Arc<TerminalSession>,
    interaction: Entity<TerminalInteraction>,
    input: Entity<TerminalInput>,
    focus_handle: FocusHandle,
    interactive: bool,
    resize_owner: bool,
    style: TerminalStyle,
}

#[derive(Clone)]
pub struct TerminalStyle {
    pub palette: TerminalPalette,
    pub font_family: SharedString,
    pub font_size: f32,
    pub app_zoom: f32,
}

impl TerminalElement {
    pub fn new(
        session: Arc<TerminalSession>,
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
            resize_owner,
            style,
        }
    }

    fn register_mouse_listeners(
        &self,
        geometry: TerminalGeometry,
        hitbox: Hitbox,
        window: &mut Window,
    ) {
        let interaction = self.interaction.clone();
        let focus_handle = self.focus_handle.clone();
        let interactive = self.interactive;
        let down_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !down_hitbox.is_hovered(window) {
                return;
            }
            focus_handle.focus(window);
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
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || interaction.read(cx).drag.is_none() {
                return;
            }
            interaction.update(cx, |interaction, interaction_cx| {
                interaction.mouse_up(event, geometry, interactive, interaction_cx);
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
        let text_system = window.text_system();
        let base_font = font(self.style.font_family.clone());
        let font_size = px(self.style.font_size);
        let font_id = text_system.resolve_font(&base_font);
        let cell_width = text_system
            .em_advance(font_id, font_size)
            .unwrap_or(px(self.style.font_size * 0.6));
        let line_height = px((self.style.font_size * LINE_HEIGHT_FACTOR).round());
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        let measured = terminal_grid_size(
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            f32::from(cell_width),
            f32::from(line_height),
        );
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
        let hovered_link = self.interaction.update(cx, |interaction, _| {
            interaction.refresh_hovered_link(
                window.modifiers(),
                geometry,
                window.mouse_position(),
                hitbox.is_hovered(window),
                self.interactive,
            );
            interaction.hovered_link.clone()
        });

        let terminal_palette = self.style.palette;
        let base = palette::base_palette_for(terminal_palette);
        let mut backgrounds: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
        let mut selections: Vec<Bounds<Pixels>> = Vec::new();
        let mut selection_end = None;
        let mut procedural = Vec::new();
        let mut spans: Vec<(usize, StyledSpan)> = Vec::new();
        let mut cursor = None;
        let mut cursor_procedural = None;
        let mut cursor_text = None;
        let mut cursor_cell = None;
        let mut cursor_cols = 1;
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
                        to_hsla(fg, 1.),
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

                let mut cell_font: Font = base_font.clone();
                if cell.flags.contains(Flags::BOLD) {
                    cell_font = cell_font.bold();
                }
                if cell.flags.contains(Flags::ITALIC) {
                    cell_font = cell_font.italic();
                }
                let underline = if link_underlined {
                    Some(gpui::UnderlineStyle {
                        color: Some(to_hsla(fg, 1.)),
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
                    color: to_hsla(fg, 1.),
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
                    }
                }
            }
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
        let selection_color = to_hsla(self.style.palette.selection, 1.);
        for quad_bounds in &prepaint.selections {
            window.paint_quad(fill(*quad_bounds, selection_color));
        }
        for cell in &prepaint.procedural {
            cell.paint(window);
        }
        for (origin, line) in &prepaint.lines {
            let _ = line.paint(*origin, prepaint.line_height, window, cx);
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
                let _ = line.paint(cursor_bounds.origin, prepaint.line_height, window, cx);
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
                                let _ = line.paint(*origin, prepaint.line_height, window, cx);
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
        let cursor_style = if prepaint.hovered_link.is_some() {
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
    use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
    use gpui::{point, px, Bounds};

    use super::{autoscroll_amount, cell_and_side, link_modifier, point_for_viewport};

    #[test]
    fn terminal_links_require_the_platform_or_control_modifier() {
        assert!(!link_modifier(gpui::Modifiers::default()));
        assert!(!link_modifier(gpui::Modifiers {
            alt: true,
            ..Default::default()
        }));
        assert!(link_modifier(gpui::Modifiers {
            platform: true,
            ..Default::default()
        }));
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
