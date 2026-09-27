use gramhive::prelude::*;
#[derive(Command)]
#[command(name = "bad")]
struct Bad {
    #[rest]
    a: String,
    b: String,
}
fn main() {}
