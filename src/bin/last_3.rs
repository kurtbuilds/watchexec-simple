use std::{process, thread};
use std::time::Duration;

fn main() {
    let mut i = 0;
    while i < 3 {
        println!("Alive! id={} i={}", process::id(), i);
        thread::sleep(Duration::from_millis(1000));
        i += 1;
    }
    println!("Done!");
}
