//! Platform-neutral half of DupliDetect: everything that must behave
//! identically to the macOS Swift implementation, with no OS or GUI
//! dependencies so it can be tested in isolation.

pub mod fingerprint;
pub mod formats;
pub mod keep;
pub mod model;
pub mod hash;
pub mod matcher;
pub mod unionfind;
