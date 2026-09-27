use gramhive::prelude::*;
#[derive(Command)]
#[command(name = "bad")]
struct Bad {
    #[rest]
    a: String,
    #[rest]
    b: String,
}
fn main() {}
