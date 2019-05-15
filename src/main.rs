use simplelog::*;
use std::fs::OpenOptions;

const PROCESSES: [(&'static str, &'static str); 2] =
    [("/usr/sbin/sshd", ""), ("/usr/sbin/haveged", "")];

fn main() {
    WriteLogger::init(
        log::LevelFilter::Trace,
        Config::default(),
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .append(true)
            .open("/log")
            .expect("Failed to open log file"),
    )
    .expect("Failed to set up logger");

    // Start reaper
    let reaper = librsinit::Reaper::new();

    reaper.spawn(&PROCESSES);
}
