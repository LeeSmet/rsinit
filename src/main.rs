use librsinit::ensure_process;
use simplelog::*;
use std::fs::OpenOptions;
use std::thread;

const PROCESSES: [(&'static str, &'static str); 2] =
    [("/usr/sbin/sshd", "-D"), ("/usr/sbin/haveged", "")];

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

    let mut handles = Vec::with_capacity(PROCESSES.len());

    for (process, args) in &PROCESSES {
        let handle = thread::spawn(move || {
            ensure_process(process, args);
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }
}
