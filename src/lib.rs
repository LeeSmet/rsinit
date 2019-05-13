#[macro_use]
extern crate log;
use std::process::Command;

pub fn ensure_process(name: &str, args: &str) -> ! {
    let args: Vec<&str> = args.split_whitespace().collect();

    loop {
        let mut cmd = Command::new(name);
        let cmd = cmd.args(&args);

        debug!("Spawning process {}", name);
        let mut child = cmd.spawn().expect("Failed to spawn process");

        info!("Spawned process {}", name);

        match child.wait() {
            Ok(code) => {
                warn!("Process {} exited with code {}", name, code);
            }
            Err(e) => {
                error!("Process {} errored: {}", name, e);
            }
        }
    }
}
