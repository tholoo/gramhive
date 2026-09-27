use gramhive::prelude::*;
#[derive(Command)]
#[command(name = "bad")]
struct Bad {
    #[rest]
    a: i64,
}
fn main() {}
