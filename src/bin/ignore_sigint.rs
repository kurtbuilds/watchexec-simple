use std::{process, thread};
use std::time::Duration;

fn main() {
    let mut i = 0;
    ctrlc::set_handler(move || {
        println!("I'm not dead yet!");
    }).expect("Error setting Ctrl-C handler");

    loop {
        println!("Alive! id={} i={}", process::id(), i);
        i += 1;
        thread::sleep(Duration::from_millis(1000));
    }

}
