use std::thread;
use std::time::Duration;

fn main() {
    loop {
        println!("Alive!");
        thread::sleep(Duration::from_millis(1000));
    }
}
