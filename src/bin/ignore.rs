use std::{process, thread};
use std::time::Duration;
use ignore::gitignore::Gitignore;

fn main() {
    let (ignore, _) = Gitignore::new(".gitignore");
    println!("{}", ignore.num_ignores());
    println!("{}", ignore.matched_path_or_any_parents("target/debug/alive", false).is_ignore())
}
