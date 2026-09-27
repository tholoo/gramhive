use gramhive::prelude::*;
#[derive(CallbackData)]
#[callback(prefix = "bad", prefix = "again")]
enum Bad {
    Unit,
}
fn main() {}
