#[macro_use]
extern crate log;

use std::process::Command;
use std::thread;

pub mod reaper;

pub fn ensure_process(name: &str, args: &str) {
    let formatted_args: Vec<&str> = args.split_whitespace().collect();

    let mut cmd = Command::new(name);
    cmd.args(&formatted_args);

    debug!("Spawning process {}", name);
    let mut child;

    match spawn_process(&mut cmd) {
        Some(ch) => child = ch,
        None => {
            error!("Failed to spawn {} ({})", name, args);
            return;
        }
    }

    info!("Spawned process {}, Pid: {}", name, child.id());
}

fn spawn_process(cmd: &mut Command) -> Option<std::process::Child> {
    // 5 batches
    for batch in 0..5 {
        // try 3 times per batch
        for _ in 0..3 {
            match cmd.spawn() {
                Err(e) => {
                    warn!("Failed to spawn command: {}", e);
                    continue;
                }
                Ok(child) => {
                    debug!("Spawned command");
                    return Some(child);
                }
            }
        }

        thread::sleep(std::time::Duration::from_secs(10 * (batch + 1)));
    }

    None
}
