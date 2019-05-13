use librsinit::ensure_process;
use std::thread;

fn main() {
    let logger = pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Trace)
        .build();

    log::set_boxed_logger(Box::new(logger)).expect("Failed to set logger");

    let handle = thread::spawn(|| {
        ensure_process("/usr/sbin/sshd", "-D");
    });

    handle.join().unwrap();
}
