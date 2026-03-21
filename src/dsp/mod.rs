//! Digital Signal Processing primitives for the guiding engine.
//!
//! All hot-path functions are allocation-free.  Filters are initialised
//! once (heap allocation permitted at construction) and then process
//! samples in a tight loop with no further allocation.

pub mod anc;
pub mod pid;
pub mod wavetable;
pub mod zdf_filter;

pub use anc::AncEngine;
pub use pid::Pid;
pub use wavetable::PeriodicErrorTable;
pub use zdf_filter::{ZdfFourPole, ZdfMorphable, ZdfOnePole, ZdfSvf, SvfOutput};
