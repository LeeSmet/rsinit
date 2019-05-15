#[macro_use]
extern crate log;

use std::process::Command;
use std::thread;

use crossbeam::channel::Receiver;

pub mod reaper;
use reaper::{Event, ReaperNotification};

pub struct ProcessMonitor<'a> {
    interested_pids: Vec<u32>,
    cmd: &'a str,
    args: &'a str,
    reaper_event_chan: Receiver<ReaperNotification>,
}

impl<'a> ProcessMonitor<'a> {
    pub fn new(
        cmd: &'a str,
        args: &'a str,
        reaper_event_chan: Receiver<ReaperNotification>,
    ) -> Self {
        ProcessMonitor {
            interested_pids: Vec::new(),
            cmd,
            args,
            reaper_event_chan,
        }
    }

    pub fn spawn(mut self) -> Result<(), ()> {
        let child = self.spawn_process()?;

        self.interested_pids.push(child.id());

        loop {
            let notification = self.reaper_event_chan.recv().unwrap();
            match notification.event {
                Event::ExitSuccess => {
                    match notification.children.len() {
                        0 => {
                            debug!("Monitored process exited successful, no children - Monitor exiting");
                            return Ok(());
                        }
                        c => {
                            debug!("Monitored process exited successful, and has {} children - continue to monitor children", c);
                            self.interested_pids = self
                                .interested_pids
                                .drain(..)
                                .filter(|p| p != &notification.pid)
                                .collect();
                            self.interested_pids.extend(notification.children);
                        }
                    }
                }
                Event::ExitCode | Event::ExitSignal => {
                    debug!("Monitored process stopped unexpectedly, restarting");
                    // TODO: handle processes with multiple children (interested_pids.len() > 1)
                    self.interested_pids.push(self.spawn_process()?.id());
                }
            }
        }
    }

    fn spawn_process(&self) -> Result<std::process::Child, ()> {
        let mut cmd = Command::new(self.cmd);
        cmd.args(self.args.split_whitespace());

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
                        debug!("Spawned command {} {}", self.cmd, self.args);
                        return Ok(child);
                    }
                }
            }

            thread::sleep(std::time::Duration::from_secs(10 * (batch + 1)));
        }

        Err(())
    }
}
