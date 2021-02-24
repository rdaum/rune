#![warn(
    clippy::trivially_copy_pass_by_ref,
    clippy::explicit_iter_loop,
    clippy::inefficient_to_string,
    clippy::missing_const_for_fn,
    clippy::needless_borrow,
    clippy::unicode_not_nfc,
)]
#[macro_use]
mod macros;
#[macro_use]
mod lisp_object;
mod arith;
mod compile;
mod error;
mod eval;
mod func;
mod gc;
mod hashmap;
mod intern;
mod opcode;
mod reader;
#[macro_use]
extern crate fn_macros;

#[allow(clippy::missing_const_for_fn)]
fn main() {
    eval::run();
}
