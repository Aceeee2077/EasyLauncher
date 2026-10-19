//! Test-only subprocess. This contains no Minecraft code and is never shipped.
use std::io::{self, BufRead, Write};
fn main() {
    if std::env::args().any(|a| a == "-version") {
        eprintln!("openjdk version \"21.0.5\" TEST FIXTURE");
        return;
    }
    println!("[00:00:00] [Server thread/INFO]: Done (0.01s)! For help, type \"help\"");
    io::stdout().flush().unwrap();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        match line.as_str() {
            "stop" => {
                println!("[00:00:01] [Server thread/INFO]: Stopping server");
                return;
            }
            "fixture-crash" => {
                eprintln!("Intentional test fixture failure");
                std::process::exit(7);
            }
            _ => println!("[00:00:00] [Server thread/INFO]: Received: {line}"),
        };
        io::stdout().flush().unwrap();
    }
}
