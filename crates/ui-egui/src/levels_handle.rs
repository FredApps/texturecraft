//! The five handles of the levels editor and how each one reads and moves a [`Levels`]

use texturecraft_engine::core::Levels;

use crate::i18n::tr;

/// Narrowest gap the handles and typed values leave between the low and high handle of a pair
const MIN_HANDLE_GAP: f32 = 0.01;

/// Gamma range of the handle and typed values: the engine's own, so the handle nearly reaches either input handle
const GAMMA_RANGE: std::ops::RangeInclusive<f32> = 0.01..=10.0;

/// One draggable value of a levels effect
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LevelsHandle {
    InputLow,
    Gamma,
    InputHigh,
    OutputLow,
    OutputHigh,
}

impl LevelsHandle {
    /// The handles above the histogram, left to right at the default levels
    pub const INPUT: [LevelsHandle; 3] = [LevelsHandle::InputLow, LevelsHandle::Gamma, LevelsHandle::InputHigh];

    pub fn label(self) -> &'static str {
        match self {
            LevelsHandle::InputLow => tr("Input low"),
            LevelsHandle::Gamma => tr("Gamma"),
            LevelsHandle::InputHigh => tr("Input high"),
            LevelsHandle::OutputLow => tr("Output low"),
            LevelsHandle::OutputHigh => tr("Output high"),
        }
    }

    pub fn value(self, levels: &Levels) -> f32 {
        match self {
            LevelsHandle::InputLow => levels.in_low,
            LevelsHandle::Gamma => levels.gamma,
            LevelsHandle::InputHigh => levels.in_high,
            LevelsHandle::OutputLow => levels.out_low,
            LevelsHandle::OutputHigh => levels.out_high,
        }
    }

    /// Where the handle sits along its track (0 left, 1 right); gamma 1 is halfway between the input handles
    pub fn position(self, levels: &Levels) -> f32 {
        match self {
            LevelsHandle::Gamma => levels.in_low + input_span(levels) * 0.5f32.powf(levels.gamma),
            _ => self.value(levels),
        }
        .clamp(0.0, 1.0)
    }

    /// Moves the handle to `position` along its track (see [`Self::position`])
    pub fn set_position(self, levels: &mut Levels, position: f32) {
        let value = match self {
            LevelsHandle::Gamma => {
                let span = input_span(levels).max(f32::EPSILON);
                let fraction = ((position - levels.in_low) / span).clamp(f32::EPSILON, 1.0 - f32::EPSILON);
                fraction.ln() / 0.5f32.ln()
            }
            _ => position,
        };
        self.set_value(levels, value);
    }

    /// Sets the handle's value without crossing a neighbour: input low < gamma < input high, outputs keep their sides
    pub fn set_value(self, levels: &mut Levels, value: f32) {
        if !value.is_finite() {
            return;
        }

        // The gap keeps each output handle on its side; min/max, not clamp, as a project's bounds can cross
        let inverted = levels.out_low > levels.out_high;
        match self {
            LevelsHandle::InputLow => levels.in_low = value.min(levels.in_high - MIN_HANDLE_GAP).max(0.0),
            LevelsHandle::Gamma => levels.gamma = value.clamp(*GAMMA_RANGE.start(), *GAMMA_RANGE.end()),
            LevelsHandle::InputHigh => levels.in_high = value.max(levels.in_low + MIN_HANDLE_GAP).min(1.0),
            LevelsHandle::OutputLow if inverted => levels.out_low = value.max(levels.out_high + MIN_HANDLE_GAP).min(1.0),
            LevelsHandle::OutputLow => levels.out_low = value.min(levels.out_high - MIN_HANDLE_GAP).max(0.0),
            LevelsHandle::OutputHigh if inverted => levels.out_high = value.min(levels.out_low - MIN_HANDLE_GAP).max(0.0),
            LevelsHandle::OutputHigh => levels.out_high = value.max(levels.out_low + MIN_HANDLE_GAP).min(1.0),
        }
    }
}

fn input_span(levels: &Levels) -> f32 {
    (levels.in_high - levels.in_low).max(0.0)
}
