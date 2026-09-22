//! Authoritative legal-record mutation and synchronized derived-index maintenance.
//!
//! `LegalState` remains the single owner. Child modules group writes by legal aggregate so
//! lifecycle/index logic stays reviewable without creating peer mutation authorities.

mod custody;
mod enforcement;
mod evidence;
mod informants;
mod investigation;
mod investigation_work;
mod prosecution;
mod witnesses;
