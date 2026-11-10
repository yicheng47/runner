#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalGridSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizePushVerdict {
    Push(TerminalGridSize),
    Unchanged,
    SuppressedUnplaced,
    SuppressedNonOwner,
}

pub fn terminal_grid_size(
    width: f32,
    height: f32,
    cell_width: f32,
    line_height: f32,
) -> Option<TerminalGridSize> {
    if width <= 0. || height <= 0. {
        return None;
    }
    Some(TerminalGridSize {
        cols: (width / cell_width).floor().max(2.) as u16,
        rows: (height / line_height).floor().max(2.) as u16,
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
    };
    const LAST: TerminalGridSize = TerminalGridSize { cols: 76, rows: 59 };

    #[test]
    fn owner_pushes_a_changed_size() {
        assert_eq!(
            size_push_verdict(Some(CURRENT), LAST, true),
            SizePushVerdict::Push(CURRENT)
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
            terminal_grid_size(0., 800., 8., 18.),
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
            terminal_grid_size(1200., 800., 8., 20.),
            Some(TerminalGridSize {
                cols: 150,
                rows: 40,
            })
        );
    }
}
