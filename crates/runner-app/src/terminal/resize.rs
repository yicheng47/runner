#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalGridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizePushVerdict {
    Push(TerminalGridSize),
    Unchanged,
    SuppressedUnplaced,
    SuppressedNonOwner,
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
}
