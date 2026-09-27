use gramhive::prelude::*;
#[derive(CallbackData)]
#[callback(prefix = "bad")]
enum Bad {
    Tuple(String),
}
fn main() {}
