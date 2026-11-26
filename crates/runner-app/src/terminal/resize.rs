/// Wire grid bounds enforced by the companion protocol (`ViewportRequest::clamped_size`).
pub const REMOTE_MIN_COLS: u16 = 20;
pub const REMOTE_MAX_COLS: u16 = 512;
pub const REMOTE_MIN_ROWS: u16 = 8;
pub const REMOTE_MAX_ROWS: u16 = 256;

/// Maximum unacknowledged pushes for the same `(epoch, clamped_size)` when a
/// remote host still reports a zero cell size. Allows recovery if a current
/// host transiently drops or rejects the first same-grid resize while keeping
/// a legacy zero-cell host from spinning across subsequent repaints.
pub const REMOTE_ZERO_CELL_MAX_ATTEMPTS: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalGridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl TerminalGridSize {
    pub fn clamped_remote(self) -> Self {
        Self {
            cols: self.cols.clamp(REMOTE_MIN_COLS, REMOTE_MAX_COLS),
            rows: self.rows.clamp(REMOTE_MIN_ROWS, REMOTE_MAX_ROWS),
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RemoteZeroCellState {
    last: Option<RemoteZeroCellAttempt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RemoteZeroCellAttempt {
    epoch: u64,
    size: TerminalGridSize,
    attempts: u8,
}

impl RemoteZeroCellState {
    pub fn clear(&mut self) {
        self.last = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizePushVerdict {
    Push(TerminalGridSize),
    Unchanged,
    SuppressedUnplaced,
    SuppressedNonOwner,
}

/// Decides whether prepaint should push a resize, clamping the compared and
/// pushed grid to the companion wire bounds for remote mirrors (while leaving
/// local measurements untouched) and combining positive legacy zero-cell
/// `Resized` confirmation with bounded retries when a remote host still
/// reports a zero cell size.
#[allow(clippy::too_many_arguments)]
pub fn resolve_size_push(
    measured: Option<TerminalGridSize>,
    session_cols: u16,
    session_rows: u16,
    session_cell: (u16, u16),
    is_remote: bool,
    resize_owner: bool,
    desktop_control: bool,
    resize_epoch: u64,
    legacy_zero_cell_confirmed: bool,
    zero_cell_state: &mut RemoteZeroCellState,
) -> SizePushVerdict {
    let effective = measured.map(|size| {
        if is_remote {
            size.clamped_remote()
        } else {
            size
        }
    });
    let same_target = |attempt: RemoteZeroCellAttempt, target: TerminalGridSize| -> bool {
        attempt.epoch == resize_epoch
            && if legacy_zero_cell_confirmed {
                attempt.size.cols == target.cols && attempt.size.rows == target.rows
            } else {
                attempt.size == target
            }
    };
    let has_cell = session_cell.0 > 0 && session_cell.1 > 0;
    let last_pushed = if has_cell {
        zero_cell_state.clear();
        TerminalGridSize {
            cols: session_cols,
            rows: session_rows,
            cell_width: session_cell.0,
            cell_height: session_cell.1,
        }
    } else if !is_remote || !resize_owner || !desktop_control {
        zero_cell_state.clear();
        TerminalGridSize {
            cols: session_cols,
            rows: session_rows,
            cell_width: 0,
            cell_height: 0,
        }
    } else if legacy_zero_cell_confirmed
        && effective
            .is_some_and(|target| target.cols == session_cols && target.rows == session_rows)
    {
        zero_cell_state.clear();
        effective.unwrap()
    } else if let Some(attempt) = zero_cell_state.last {
        if let Some(target) = effective.filter(|target| same_target(attempt, *target)) {
            if attempt.attempts >= REMOTE_ZERO_CELL_MAX_ATTEMPTS {
                target
            } else {
                TerminalGridSize {
                    cols: session_cols,
                    rows: session_rows,
                    cell_width: 0,
                    cell_height: 0,
                }
            }
        } else {
            zero_cell_state.clear();
            TerminalGridSize {
                cols: session_cols,
                rows: session_rows,
                cell_width: 0,
                cell_height: 0,
            }
        }
    } else {
        TerminalGridSize {
            cols: session_cols,
            rows: session_rows,
            cell_width: 0,
            cell_height: 0,
        }
    };

    let verdict = size_push_verdict(effective, last_pushed, resize_owner);
    if let SizePushVerdict::Push(size) = verdict {
        if is_remote && desktop_control && !has_cell {
            zero_cell_state.last = match zero_cell_state.last {
                Some(prev) if same_target(prev, size) => Some(RemoteZeroCellAttempt {
                    epoch: resize_epoch,
                    size,
                    attempts: prev.attempts.saturating_add(1),
                }),
                _ => Some(RemoteZeroCellAttempt {
                    epoch: resize_epoch,
                    size,
                    attempts: 1,
                }),
            };
        }
    }
    verdict
}

/// Converts logical cell metrics and display scale into device pixels, clamped
/// to the Kitty/wire range of `1..=1024` pixels per side.
pub fn device_cell_size(cell_width: f32, line_height: f32, scale_factor: f32) -> (u16, u16) {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let width = (cell_width * scale).round().clamp(1.0, 1024.0) as u16;
    let height = (line_height * scale).round().clamp(1.0, 1024.0) as u16;
    (width, height)
}

pub fn terminal_grid_size(
    width: f32,
    height: f32,
    cell_width: f32,
    line_height: f32,
    scale_factor: f32,
) -> Option<TerminalGridSize> {
    if width <= 0. || height <= 0. {
        return None;
    }
    let (cell_px_w, cell_px_h) = device_cell_size(cell_width, line_height, scale_factor);
    Some(TerminalGridSize {
        cols: (width / cell_width).floor().max(2.) as u16,
        rows: (height / line_height).floor().max(2.) as u16,
        cell_width: cell_px_w,
        cell_height: cell_px_h,
    })
}

pub fn size_push_verdict(
    current: Option<TerminalGridSize>,
    last_pushed: TerminalGridSize,
    owner: bool,
) -> SizePushVerdict {
    let Some(current) = current else {
        return SizePushVerdict::SuppressedUnplaced;
    };
    if current == last_pushed {
        return SizePushVerdict::Unchanged;
    }
    if !owner {
        return SizePushVerdict::SuppressedNonOwner;
    }
    SizePushVerdict::Push(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: TerminalGridSize = TerminalGridSize {
        cols: 209,
        rows: 59,
        cell_width: 16,
        cell_height: 36,
    };
    const LAST: TerminalGridSize = TerminalGridSize {
        cols: 76,
        rows: 59,
        cell_width: 16,
        cell_height: 36,
    };

    #[test]
    fn owner_pushes_a_changed_size() {
        assert_eq!(
            size_push_verdict(Some(CURRENT), LAST, true),
            SizePushVerdict::Push(CURRENT)
        );
    }

    #[test]
    fn scale_only_change_pushes_new_cell_size() {
        let retina = terminal_grid_size(1200., 800., 8., 20., 2.0).unwrap();
        let standard = terminal_grid_size(1200., 800., 8., 20., 1.0).unwrap();
        assert_eq!((retina.cols, retina.rows), (standard.cols, standard.rows));
        assert_eq!((retina.cell_width, retina.cell_height), (16, 40));
        assert_eq!((standard.cell_width, standard.cell_height), (8, 20));
        assert_eq!(
            size_push_verdict(Some(standard), retina, true),
            SizePushVerdict::Push(standard)
        );
        assert_eq!(
            size_push_verdict(Some(standard), retina, false),
            SizePushVerdict::SuppressedNonOwner
        );
    }

    #[test]
    fn non_owner_neither_refits_nor_pushes() {
        assert_eq!(
            size_push_verdict(Some(CURRENT), LAST, false),
            SizePushVerdict::SuppressedNonOwner
        );
    }

    #[test]
    fn unchanged_size_dedupes_before_ownership() {
        assert_eq!(
            size_push_verdict(Some(CURRENT), CURRENT, false),
            SizePushVerdict::Unchanged
        );
    }

    #[test]
    fn unplaced_pane_has_no_grid_size() {
        assert_eq!(
            terminal_grid_size(0., 800., 8., 18., 2.0),
            None,
            "zero-width layout must not become a plausible PTY fit"
        );
        assert_eq!(
            size_push_verdict(None, LAST, true),
            SizePushVerdict::SuppressedUnplaced
        );
    }

    #[test]
    fn placed_pane_uses_its_real_body_geometry() {
        assert_eq!(
            terminal_grid_size(1200., 800., 8., 20., 2.0),
            Some(TerminalGridSize {
                cols: 150,
                rows: 40,
                cell_width: 16,
                cell_height: 40,
            })
        );
    }

    #[test]
    fn device_cell_size_clamps_to_protocol_bounds() {
        assert_eq!(device_cell_size(0.1, 0.2, 1.0), (1, 1));
        assert_eq!(device_cell_size(800.0, 900.0, 2.0), (1024, 1024));
        assert_eq!(device_cell_size(8.4, 18.0, 2.0), (17, 36));
        assert_eq!(device_cell_size(8.0, 18.0, 0.0), (8, 18));
    }

    #[test]
    fn remote_resize_clamps_both_bounds_recovers_from_failure_and_bounds_legacy_hosts() {
        fn run_sim(
            cols: u16,
            rows: u16,
            legacy: bool,
            drop_first: bool,
            prepaints: usize,
        ) -> (usize, (u16, u16), (u16, u16)) {
            let mut host_grid = (
                cols.clamp(REMOTE_MIN_COLS, REMOTE_MAX_COLS),
                rows.clamp(REMOTE_MIN_ROWS, REMOTE_MAX_ROWS),
            );
            let mut host_cell = (0u16, 0u16);
            let mut state = RemoteZeroCellState::default();
            let mut sends = 0usize;
            let measured = Some(TerminalGridSize {
                cols,
                rows,
                cell_width: 9,
                cell_height: 18,
            });
            for _ in 0..prepaints {
                if let SizePushVerdict::Push(size) = resolve_size_push(
                    measured,
                    host_grid.0,
                    host_grid.1,
                    host_cell,
                    true,
                    true,
                    true,
                    1,
                    false,
                    &mut state,
                ) {
                    sends += 1;
                    assert!(
                        (REMOTE_MIN_COLS..=REMOTE_MAX_COLS).contains(&size.cols)
                            && (REMOTE_MIN_ROWS..=REMOTE_MAX_ROWS).contains(&size.rows),
                        "remote push must use clamped wire grid: {size:?}"
                    );
                    if drop_first && sends == 1 {
                        continue;
                    }
                    host_grid = (
                        size.cols.clamp(REMOTE_MIN_COLS, REMOTE_MAX_COLS),
                        size.rows.clamp(REMOTE_MIN_ROWS, REMOTE_MAX_ROWS),
                    );
                    if !legacy {
                        host_cell = (size.cell_width, size.cell_height);
                    }
                }
            }
            (sends, host_grid, host_cell)
        }

        // Modern host at lower clamp (10x6 -> 20x8) and upper clamp (520x260 -> 512x256):
        // single push across 5 prepaints, zero spin.
        assert_eq!(run_sim(10, 6, false, false, 5), (1, (20, 8), (9, 18)));
        assert_eq!(run_sim(520, 260, false, false, 5), (1, (512, 256), (9, 18)));

        // Modern host dropping its first same-grid viewport recovers on retry at normal
        // and clamped sizes, then stops.
        assert_eq!(run_sim(80, 24, false, true, 5), (2, (80, 24), (9, 18)));
        assert_eq!(run_sim(10, 6, false, true, 5), (2, (20, 8), (9, 18)));
        assert_eq!(run_sim(520, 260, false, true, 5), (2, (512, 256), (9, 18)));

        // Legacy zero-cell host is strictly bounded at REMOTE_ZERO_CELL_MAX_ATTEMPTS
        // across 10 prepaints at normal and both clamped boundaries.
        let cap = usize::from(REMOTE_ZERO_CELL_MAX_ATTEMPTS);
        assert_eq!(run_sim(80, 24, true, false, 10), (cap, (80, 24), (0, 0)));
        assert_eq!(run_sim(10, 6, true, false, 10), (cap, (20, 8), (0, 0)));
        assert_eq!(
            run_sim(520, 260, true, false, 10),
            (cap, (512, 256), (0, 0))
        );

        // Legacy host that explicitly answers a non-zero resize with `Resized { cell: 0x0, Desktop }`
        // stops immediately for unchanged `(cols, rows)` while still pushing real grid changes.
        let mut ack_state = RemoteZeroCellState::default();
        let grid_101 = TerminalGridSize {
            cols: 101,
            rows: 30,
            cell_width: 16,
            cell_height: 36,
        };
        assert_eq!(
            resolve_size_push(
                Some(grid_101),
                80,
                24,
                (0, 0),
                true,
                true,
                true,
                1,
                false,
                &mut ack_state,
            ),
            SizePushVerdict::Push(grid_101)
        );
        // Host answers with `(101, 30, cell: 0x0, Desktop)` -> legacy_zero_cell_confirmed = true.
        // Same-grid and scale-only changes produce zero pushes.
        for cell in [(16, 36), (8, 18)] {
            let scaled = TerminalGridSize {
                cols: 101,
                rows: 30,
                cell_width: cell.0,
                cell_height: cell.1,
            };
            for _ in 0..5 {
                assert_eq!(
                    resolve_size_push(
                        Some(scaled),
                        101,
                        30,
                        (0, 0),
                        true,
                        true,
                        true,
                        1,
                        true,
                        &mut ack_state,
                    ),
                    SizePushVerdict::Unchanged
                );
            }
        }
        // When a confirmed legacy peer's target grid changes (101x30 -> 110x30) and the
        // host acknowledgement is delayed, pushes stay capped at REMOTE_ZERO_CELL_MAX_ATTEMPTS
        // across 10 prepaints, a display-scale-only change does NOT restart the pending work,
        // and once the delayed zero-cell ack lands at 110x30 further prepaints send nothing.
        let grid_110 = TerminalGridSize {
            cols: 110,
            rows: 30,
            cell_width: 16,
            cell_height: 36,
        };
        let legacy_grid_pushes: usize = (0..10)
            .filter(|_| {
                matches!(
                    resolve_size_push(
                        Some(grid_110),
                        101,
                        30,
                        (0, 0),
                        true,
                        true,
                        true,
                        1,
                        true,
                        &mut ack_state,
                    ),
                    SizePushVerdict::Push(pushed) if pushed == grid_110
                )
            })
            .count();
        assert_eq!(
            legacy_grid_pushes, cap,
            "confirmed legacy peer with delayed grid ack must cap identical pushes"
        );
        let grid_110_rescaled = TerminalGridSize {
            cols: 110,
            rows: 30,
            cell_width: 8,
            cell_height: 18,
        };
        for _ in 0..5 {
            assert_eq!(
                resolve_size_push(
                    Some(grid_110_rescaled),
                    101,
                    30,
                    (0, 0),
                    true,
                    true,
                    true,
                    1,
                    true,
                    &mut ack_state,
                ),
                SizePushVerdict::Unchanged,
                "scale-only change must not restart pending grid work on a confirmed legacy peer"
            );
        }
        // Delayed zero-cell ack arrives for 110x30 -> further prepaints send zero pushes.
        for _ in 0..10 {
            assert_eq!(
                resolve_size_push(
                    Some(grid_110_rescaled),
                    110,
                    30,
                    (0, 0),
                    true,
                    true,
                    true,
                    1,
                    true,
                    &mut ack_state,
                ),
                SizePushVerdict::Unchanged
            );
        }

        // Delayed/silent zero-cell peer starting at a DIFFERENT grid (80x24) while the
        // pane requests 20x8: retries stay capped at REMOTE_ZERO_CELL_MAX_ATTEMPTS across
        // 10 prepaints, a genuinely new target or new epoch pushes fresh, and a delayed
        // non-zero acknowledgement settles cleanly to Unchanged.
        let mut delayed_state = RemoteZeroCellState::default();
        let narrow = TerminalGridSize {
            cols: 20,
            rows: 8,
            cell_width: 9,
            cell_height: 18,
        };
        let pushes: usize = (0..10)
            .filter(|_| {
                matches!(
                    resolve_size_push(
                        Some(narrow),
                        80,
                        24,
                        (0, 0),
                        true,
                        true,
                        true,
                        1,
                        false,
                        &mut delayed_state,
                    ),
                    SizePushVerdict::Push(_)
                )
            })
            .count();
        assert_eq!(
            pushes, cap,
            "delayed zero-cell peer at a different grid must cap identical pushes"
        );
        // A genuinely new requested size resets the cap and pushes.
        let wider = TerminalGridSize {
            cols: 40,
            rows: 12,
            cell_width: 9,
            cell_height: 18,
        };
        assert_eq!(
            resolve_size_push(
                Some(wider),
                80,
                24,
                (0, 0),
                true,
                true,
                true,
                1,
                false,
                &mut delayed_state,
            ),
            SizePushVerdict::Push(wider)
        );
        // An ownership/reconnect epoch bump pushes again for the same target.
        assert_eq!(
            resolve_size_push(
                Some(wider),
                80,
                24,
                (0, 0),
                true,
                true,
                true,
                2,
                false,
                &mut delayed_state,
            ),
            SizePushVerdict::Push(wider)
        );
        // Delayed peer finally acknowledges `(40, 12, 9, 18)` -> Unchanged forever.
        for _ in 0..5 {
            assert_eq!(
                resolve_size_push(
                    Some(wider),
                    40,
                    12,
                    (9, 18),
                    true,
                    true,
                    true,
                    2,
                    false,
                    &mut delayed_state,
                ),
                SizePushVerdict::Unchanged
            );
        }

        // Local mirrors do not clamp to remote bounds and never latch zero-cell retries.
        let mut local_state = RemoteZeroCellState::default();
        let small = TerminalGridSize {
            cols: 10,
            rows: 6,
            cell_width: 9,
            cell_height: 18,
        };
        for _ in 0..4 {
            assert_eq!(
                resolve_size_push(
                    Some(small),
                    10,
                    6,
                    (0, 0),
                    false,
                    true,
                    true,
                    1,
                    false,
                    &mut local_state,
                ),
                SizePushVerdict::Push(small)
            );
        }
    }
}
