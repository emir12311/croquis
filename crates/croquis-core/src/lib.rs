//! Pure circuit-core pieces, independent of the UI, the engine and the catalogue
//! storage. The catalogue is supplied through traits so this crate can be tested
//! before the full part registry is ported.

pub mod catalogue;
pub mod diagram;
pub mod document;
pub mod netlist;
pub mod route;
pub mod shift_register;
pub mod stepper;
