use librsinit::PersistentCommand;
use simplelog::*;
use std::fs::OpenOptions;

const PROCESSES: [(&'static str, &'static str); 2] =
    [("/usr/sbin/sshd", ""), ("/usr/sbin/haveged", "")];

fn main() {
    CombinedLogger::init(vec![
        TermLogger::new(log::LevelFilter::Debug, Config::default()).unwrap(),
        WriteLogger::new(
            log::LevelFilter::Trace,
            Config::default(),
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .append(true)
                .open("/log")
                .expect("Failed to open log file"),
        ),
    ])
    .expect("Failed to set up logger");

    let mut persistent_commands = Vec::with_capacity(PROCESSES.len());
    for (cmd, args) in &PROCESSES {
        persistent_commands.push(
            PersistentCommand::new(cmd, args)
                .spawn_limit(10)
                .restart_on_error(true)
                .restart_on_signal(true)
                .restart_on_success(true),
        );
    }
    // Start reaper
    let reaper = librsinit::Reaper::new();

    reaper.spawn(persistent_commands);
}
