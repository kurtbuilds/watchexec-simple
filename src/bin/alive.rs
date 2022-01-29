use std::{process, thread};
use std::time::Duration;

fn main() {
    let mut i = 0;
    loop {
        println!("Alive! id={} i={}", process::id(), i);
        i += 1;
        thread::sleep(Duration::from_millis(1000));
    }

}
