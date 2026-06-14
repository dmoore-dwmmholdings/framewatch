//! The `framewatch gui`: a window picker, live preview, and ROI editor built
//! with `eframe`/`egui`. Enabled by the `gui` feature.

mod app;

use crate::config::Config;
use crate::error::Error;

/// Launch the GUI, optionally seeded with a base [`Config`].
pub fn run(initial: Option<Config>) -> Result<(), Error> {
    app::run(initial)
}
