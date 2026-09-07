use super::*;

pub(super) type AlignmentGridRows = Vec<Vec<(u8, u8)>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AlignmentGridShortcut {
    Up,
    Right,
    Down,
    Left,
    SetTopEdge,
    SetRightEdge,
    SetBottomEdge,
    SetLeftEdge,
    ToggleSpacing,
    ToggleBaseline,
}

impl AlignmentGridShortcut {
    pub(super) fn from_key_down(event: &KeyDownEvent) -> Option<Self> {
        if event.keystroke.modifiers.modified() {
            return None;
        }
        match event.keystroke.key.as_str() {
            "up" => Some(Self::Up),
            "right" => Some(Self::Right),
            "down" => Some(Self::Down),
            "left" => Some(Self::Left),
            "w" => Some(Self::SetTopEdge),
            "d" => Some(Self::SetRightEdge),
            "s" => Some(Self::SetBottomEdge),
            "a" => Some(Self::SetLeftEdge),
            "x" => Some(Self::ToggleSpacing),
            "b" => Some(Self::ToggleBaseline),
            _ => None,
        }
    }

    const fn alignment_delta(self) -> Option<(i8, i8)> {
        match self {
            Self::Up => Some((0, -1)),
            Self::Right => Some((1, 0)),
            Self::Down => Some((0, 1)),
            Self::Left => Some((-1, 0)),
            Self::SetTopEdge
            | Self::SetRightEdge
            | Self::SetBottomEdge
            | Self::SetLeftEdge
            | Self::ToggleSpacing
            | Self::ToggleBaseline => None,
        }
    }
}

pub(super) fn alignment_grid_candidates(layout: &DesignLayout) -> (AlignmentGridRows, f32, f32) {
    match (layout.mode, layout.item_spacing_mode) {
        (DesignLayoutMode::Horizontal, DesignItemSpacingMode::Auto) => (
            (0..3)
                .map(|y| vec![(layout.alignment_x, y)])
                .collect::<Vec<_>>(),
            28.,
            72.,
        ),
        (DesignLayoutMode::Vertical, DesignItemSpacingMode::Auto) => (
            vec![(0..3).map(|x| (x, layout.alignment_y)).collect::<Vec<_>>()],
            88.,
            28.,
        ),
        (
            DesignLayoutMode::Horizontal | DesignLayoutMode::Vertical,
            DesignItemSpacingMode::Fixed,
        ) => (
            (0..3)
                .map(|y| (0..3).map(|x| (x, y)).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            88.,
            72.,
        ),
        (DesignLayoutMode::None | DesignLayoutMode::Grid, _) => (Vec::new(), 0., 0.),
    }
}

pub(super) fn alignment_for_shortcut(
    layout: &DesignLayout,
    shortcut: AlignmentGridShortcut,
) -> Option<DesignAutoLayoutAlignment> {
    if !matches!(
        layout.mode,
        DesignLayoutMode::Horizontal | DesignLayoutMode::Vertical
    ) {
        return None;
    }
    let mut x = layout.alignment_x;
    let mut y = layout.alignment_y;
    if let Some((delta_x, delta_y)) = shortcut.alignment_delta() {
        if layout.item_spacing_mode == DesignItemSpacingMode::Auto
            && ((layout.mode == DesignLayoutMode::Horizontal && delta_x != 0)
                || (layout.mode == DesignLayoutMode::Vertical && delta_y != 0))
        {
            return None;
        }
        x = i16::from(x).saturating_add(i16::from(delta_x)).clamp(0, 2) as u8;
        y = i16::from(y).saturating_add(i16::from(delta_y)).clamp(0, 2) as u8;
    } else {
        match shortcut {
            AlignmentGridShortcut::SetTopEdge
                if layout.item_spacing_mode != DesignItemSpacingMode::Auto
                    || layout.mode != DesignLayoutMode::Vertical =>
            {
                y = 0;
            }
            AlignmentGridShortcut::SetRightEdge
                if layout.item_spacing_mode != DesignItemSpacingMode::Auto
                    || layout.mode != DesignLayoutMode::Horizontal =>
            {
                x = 2;
            }
            AlignmentGridShortcut::SetBottomEdge
                if layout.item_spacing_mode != DesignItemSpacingMode::Auto
                    || layout.mode != DesignLayoutMode::Vertical =>
            {
                y = 2;
            }
            AlignmentGridShortcut::SetLeftEdge
                if layout.item_spacing_mode != DesignItemSpacingMode::Auto
                    || layout.mode != DesignLayoutMode::Horizontal =>
            {
                x = 0;
            }
            AlignmentGridShortcut::SetTopEdge
            | AlignmentGridShortcut::SetRightEdge
            | AlignmentGridShortcut::SetBottomEdge
            | AlignmentGridShortcut::SetLeftEdge
            | AlignmentGridShortcut::ToggleSpacing
            | AlignmentGridShortcut::ToggleBaseline => return None,
            AlignmentGridShortcut::Up
            | AlignmentGridShortcut::Right
            | AlignmentGridShortcut::Down
            | AlignmentGridShortcut::Left => unreachable!("arrows have an alignment delta"),
        }
    }
    (x != layout.alignment_x || y != layout.alignment_y)
        .then_some(DesignAutoLayoutAlignment::new(x, y))
}
