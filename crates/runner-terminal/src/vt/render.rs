//! Safe `!Send`/`!Sync` wrapper around `GhosttyRenderState` and its reusable
//! row/cell iterators.

use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr;

use ghostty_vt_sys as sys;

use super::terminal::{CellInfo, RowInfo, Style, Terminal};
use super::{check, Result};
use crate::palette::Rgb;
use crate::types::CursorShape;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RenderDirty {
    Clean,
    Partial,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RenderCursorInfo {
    pub(crate) viewport_pos: Option<(u16, u16)>,
    pub(crate) wide_tail: bool,
    pub(crate) visible: bool,
    pub(crate) blinking: bool,
    pub(crate) shape: CursorShape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RenderColors {
    pub(crate) background: Rgb,
    pub(crate) foreground: Rgb,
    pub(crate) cursor: Option<Rgb>,
    pub(crate) palette: [Rgb; 256],
}

#[derive(Clone, Debug)]
pub(crate) struct RenderCell {
    pub(crate) info: CellInfo,
    pub(crate) style: Style,
    pub(crate) graphemes: Vec<char>,
    pub(crate) fg_resolved: Option<Rgb>,
    pub(crate) bg_resolved: Option<Rgb>,
    pub(crate) selected: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RenderRow {
    pub(crate) viewport_y: i32,
    pub(crate) dirty: bool,
    pub(crate) info: RowInfo,
    pub(crate) selection: Option<(u16, u16)>,
    pub(crate) cells: Vec<RenderCell>,
}

/// Reusable render-state handle with pre-allocated row and cell iterators so
/// viewport updates do not allocate iterator handles per frame.
pub(crate) struct RenderState {
    raw: sys::GhosttyRenderState,
    row_iter: sys::GhosttyRenderStateRowIterator,
    row_cells: sys::GhosttyRenderStateRowCells,
    #[cfg(test)]
    update_count: usize,
    _not_send_sync: PhantomData<*mut ()>,
}

impl RenderState {
    pub(crate) fn new() -> Result<Self> {
        let mut raw: sys::GhosttyRenderState = ptr::null_mut();
        check(unsafe { sys::ghostty_render_state_new(ptr::null(), &mut raw) })?;
        let mut row_iter: sys::GhosttyRenderStateRowIterator = ptr::null_mut();
        if let Err(err) =
            check(unsafe { sys::ghostty_render_state_row_iterator_new(ptr::null(), &mut row_iter) })
        {
            unsafe { sys::ghostty_render_state_free(raw) };
            return Err(err);
        }
        let mut row_cells: sys::GhosttyRenderStateRowCells = ptr::null_mut();
        if let Err(err) =
            check(unsafe { sys::ghostty_render_state_row_cells_new(ptr::null(), &mut row_cells) })
        {
            unsafe {
                sys::ghostty_render_state_row_iterator_free(row_iter);
                sys::ghostty_render_state_free(raw);
            }
            return Err(err);
        }
        Ok(Self {
            raw,
            row_iter,
            row_cells,
            #[cfg(test)]
            update_count: 0,
            _not_send_sync: PhantomData,
        })
    }

    pub(crate) fn update(&mut self, term: &Terminal) -> Result<()> {
        #[cfg(test)]
        {
            self.update_count += 1;
        }
        check(unsafe { sys::ghostty_render_state_update(self.raw, term.raw()) })
    }

    #[cfg(test)]
    pub(crate) fn update_count(&self) -> usize {
        self.update_count
    }

    pub(crate) fn clean(&mut self) -> Result<()> {
        check(unsafe { sys::ghostty_render_state_clean(self.raw) })
    }

    pub(crate) fn cols(&self) -> u16 {
        let mut cols: u16 = 0;
        unsafe {
            sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_COLS,
                (&raw mut cols).cast(),
            );
        }
        cols
    }

    pub(crate) fn rows(&self) -> u16 {
        let mut rows: u16 = 0;
        unsafe {
            sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_ROWS,
                (&raw mut rows).cast(),
            );
        }
        rows
    }

    pub(crate) fn dirty(&self) -> RenderDirty {
        let mut dirty: sys::GhosttyRenderStateDirty = sys::GHOSTTY_RENDER_STATE_DIRTY_FALSE;
        unsafe {
            sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_DIRTY,
                (&raw mut dirty).cast(),
            );
        }
        match dirty {
            sys::GHOSTTY_RENDER_STATE_DIRTY_PARTIAL => RenderDirty::Partial,
            sys::GHOSTTY_RENDER_STATE_DIRTY_FULL => RenderDirty::Full,
            _ => RenderDirty::Clean,
        }
    }

    pub(crate) fn cursor(&self) -> Result<RenderCursorInfo> {
        let mut raw = sys::GhosttyRenderStateCursor {
            size: size_of::<sys::GhosttyRenderStateCursor>(),
            ..Default::default()
        };
        check(unsafe {
            sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_CURSOR,
                (&raw mut raw).cast(),
            )
        })?;
        let shape = match raw.visual_style {
            sys::GHOSTTY_CURSOR_STYLE_BAR => CursorShape::Beam,
            sys::GHOSTTY_CURSOR_STYLE_UNDERLINE => CursorShape::Underline,
            sys::GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW => CursorShape::HollowBlock,
            _ => CursorShape::Block,
        };
        Ok(RenderCursorInfo {
            viewport_pos: raw
                .viewport_has_value
                .then_some((raw.viewport_x, raw.viewport_y)),
            wide_tail: raw.viewport_has_value && raw.wide_tail,
            visible: raw.visible,
            blinking: raw.blinking,
            shape,
        })
    }

    pub(crate) fn colors(&self) -> Result<RenderColors> {
        let mut raw = sys::GhosttyRenderStateColors {
            size: size_of::<sys::GhosttyRenderStateColors>(),
            ..Default::default()
        };
        check(unsafe {
            sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_COLORS,
                (&raw mut raw).cast(),
            )
        })?;
        Ok(RenderColors {
            background: Rgb {
                r: raw.background.r,
                g: raw.background.g,
                b: raw.background.b,
            },
            foreground: Rgb {
                r: raw.foreground.r,
                g: raw.foreground.g,
                b: raw.foreground.b,
            },
            cursor: raw.cursor_has_value.then_some(Rgb {
                r: raw.cursor.r,
                g: raw.cursor.g,
                b: raw.cursor.b,
            }),
            palette: std::array::from_fn(|i| Rgb {
                r: raw.palette[i].r,
                g: raw.palette[i].g,
                b: raw.palette[i].b,
            }),
        })
    }

    pub(crate) fn collect_rows(&mut self) -> Result<Vec<RenderRow>> {
        let cols = usize::from(self.cols());
        let rows = usize::from(self.rows());
        check(unsafe {
            sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                (&raw mut self.row_iter).cast(),
            )
        })?;
        let mut out = Vec::with_capacity(rows);
        let mut grapheme_buf = Vec::<u32>::new();
        while unsafe { sys::ghostty_render_state_row_iterator_next(self.row_iter) } {
            let mut dirty = false;
            let mut raw_row: sys::GhosttyRow = 0;
            let mut viewport_y: i32 = 0;
            let mut row_sel = sys::GhosttyRenderStateRowSelection {
                size: size_of::<sys::GhosttyRenderStateRowSelection>(),
                start_x: 0,
                end_x: 0,
            };
            unsafe {
                sys::ghostty_render_state_row_get(
                    self.row_iter,
                    sys::GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY,
                    (&raw mut dirty).cast(),
                );
                sys::ghostty_render_state_row_get(
                    self.row_iter,
                    sys::GHOSTTY_RENDER_STATE_ROW_DATA_RAW,
                    (&raw mut raw_row).cast(),
                );
                sys::ghostty_render_state_row_get(
                    self.row_iter,
                    sys::GHOSTTY_RENDER_STATE_ROW_DATA_VIEWPORT_Y,
                    (&raw mut viewport_y).cast(),
                );
            }
            let sel_rc = unsafe {
                sys::ghostty_render_state_row_get(
                    self.row_iter,
                    sys::GHOSTTY_RENDER_STATE_ROW_DATA_SELECTION,
                    (&raw mut row_sel).cast(),
                )
            };
            let selection =
                (sel_rc == sys::GHOSTTY_SUCCESS).then_some((row_sel.start_x, row_sel.end_x));

            check(unsafe {
                sys::ghostty_render_state_row_get(
                    self.row_iter,
                    sys::GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                    (&raw mut self.row_cells).cast(),
                )
            })?;

            let mut cells = Vec::with_capacity(cols);
            while unsafe { sys::ghostty_render_state_row_cells_next(self.row_cells) } {
                let mut raw_cell: sys::GhosttyCell = 0;
                let mut raw_style = sys::GhosttyStyle::default();
                let mut graphemes_len: u32 = 0;
                let mut selected = false;
                let mut fg_raw = sys::GhosttyColorRgb::default();
                let mut bg_raw = sys::GhosttyColorRgb::default();
                unsafe {
                    sys::ghostty_render_state_row_cells_get(
                        self.row_cells,
                        sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
                        (&raw mut raw_cell).cast(),
                    );
                    sys::ghostty_render_state_row_cells_get(
                        self.row_cells,
                        sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                        (&raw mut raw_style).cast(),
                    );
                    sys::ghostty_render_state_row_cells_get(
                        self.row_cells,
                        sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                        (&raw mut graphemes_len).cast(),
                    );
                    sys::ghostty_render_state_row_cells_get(
                        self.row_cells,
                        sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_SELECTED,
                        (&raw mut selected).cast(),
                    );
                }
                let fg_rc = unsafe {
                    sys::ghostty_render_state_row_cells_get(
                        self.row_cells,
                        sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
                        (&raw mut fg_raw).cast(),
                    )
                };
                let bg_rc = unsafe {
                    sys::ghostty_render_state_row_cells_get(
                        self.row_cells,
                        sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
                        (&raw mut bg_raw).cast(),
                    )
                };
                let graphemes = if graphemes_len > 0 {
                    if grapheme_buf.len() < graphemes_len as usize {
                        grapheme_buf.resize(graphemes_len as usize, 0);
                    }
                    let mut cp_buf = sys::GhosttyCodepointBuffer {
                        ptr: grapheme_buf.as_mut_ptr(),
                        cap: grapheme_buf.len(),
                        len: 0,
                    };
                    let rc = unsafe {
                        sys::ghostty_render_state_row_cells_get(
                            self.row_cells,
                            sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                            (&raw mut cp_buf).cast(),
                        )
                    };
                    match rc {
                        sys::GHOSTTY_SUCCESS => {}
                        sys::GHOSTTY_OUT_OF_SPACE => {
                            grapheme_buf.resize(cp_buf.len, 0);
                            cp_buf = sys::GhosttyCodepointBuffer {
                                ptr: grapheme_buf.as_mut_ptr(),
                                cap: grapheme_buf.len(),
                                len: 0,
                            };
                            check(unsafe {
                                sys::ghostty_render_state_row_cells_get(
                                    self.row_cells,
                                    sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                                    (&raw mut cp_buf).cast(),
                                )
                            })?;
                        }
                        other => return Err(check(other).unwrap_err()),
                    }
                    grapheme_buf[..cp_buf.len.min(grapheme_buf.len())]
                        .iter()
                        .copied()
                        .filter_map(char::from_u32)
                        .collect()
                } else {
                    Vec::new()
                };
                cells.push(RenderCell {
                    info: CellInfo::from_raw(raw_cell),
                    style: Style::from_raw(raw_style),
                    graphemes,
                    fg_resolved: (fg_rc == sys::GHOSTTY_SUCCESS).then_some(Rgb {
                        r: fg_raw.r,
                        g: fg_raw.g,
                        b: fg_raw.b,
                    }),
                    bg_resolved: (bg_rc == sys::GHOSTTY_SUCCESS).then_some(Rgb {
                        r: bg_raw.r,
                        g: bg_raw.g,
                        b: bg_raw.b,
                    }),
                    selected,
                });
            }
            out.push(RenderRow {
                viewport_y,
                dirty,
                info: RowInfo::from_raw(raw_row),
                selection,
                cells,
            });
        }
        Ok(out)
    }
}

impl Drop for RenderState {
    fn drop(&mut self) {
        unsafe {
            sys::ghostty_render_state_row_cells_free(self.row_cells);
            sys::ghostty_render_state_row_iterator_free(self.row_iter);
            sys::ghostty_render_state_free(self.raw);
        }
    }
}
