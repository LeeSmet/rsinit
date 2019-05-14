#[macro_use]
extern crate log;
use std::process::Command;
use std::thread;

pub fn ensure_process(name: &str, args: &str) {
    let formatted_args: Vec<&str> = args.split_whitespace().collect();

    // try 3 times, then back of {
    loop {
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

        info!("Spawned process {}", name);

        match child.wait() {
            Ok(code) => match code.success() {
                true => {
                    info!(
                        "Process {} exited with code 0, assuming it forked itself",
                        name
                    );
                    return;
                }
                _ => warn!("Process {} exited with code {}", name, code),
            },
            Err(e) => {
                error!("Process {} errored: {}", name, e);
            }
        }
    }
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
