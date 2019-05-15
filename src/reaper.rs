use std::collections::HashMap;
use std::fmt;
use std::fs::{read_dir, File};
use std::io::Read;
use std::time::Duration;
use std::time::Instant;

use nix::sys::signal::kill;
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

use signal::trap::Trap;
use signal::Signal::*;

#[derive(Clone, Debug)]
pub struct Carcass {
    pub pid: Pid,
    pub status: Option<i32>,
    pub signal: Option<Signal>,
}

impl fmt::Display for Carcass {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match (self.status, self.signal) {
            (Some(st), None) => write!(f, "(pid={},exit={})", self.pid, st),
            (None, Some(sig)) => write!(f, "(pid={},sig={:?})", self.pid, sig),
            _ => unreachable!(),
        }
    }
}

/// reap executes waitpid, returning a zombie process ready to be reaped. This means it can't be
/// used to wait for a specific pid to exit. If there is currently no zombie process, None is returned,
/// else it returns a Carcass with information on how the process was terminated.
pub fn reap() -> Option<Carcass> {
    match waitpid(None, Some(WaitPidFlag::WNOHANG)).unwrap() {
        WaitStatus::Exited(pid, st) => Some(Carcass {
            pid,
            status: Some(st),
            signal: None,
        }),
        WaitStatus::Signaled(pid, sig, _) => Some(Carcass {
            pid,
            status: None,
            signal: Some(sig),
        }),
        WaitStatus::StillAlive => None,
        ws => {
            debug!("uninterpreted waitpid status: {:?}", ws);
            None
        }
    }
}

/// List all children of the process by looping over the /proc directory and reading the stat
/// entry. A child is identified as a process which has the given PID as 4th entry in the stat file
/// in the process id directory.
fn list_children(parent: Pid) -> Vec<Pid> {
    read_dir("/proc")
        .expect("unable to list /proc")
        .filter_map(|rde| {
            rde.ok().and_then(|de| {
                de.file_name()
                    .to_str()
                    .and_then(|fname| str::parse(fname).ok())
                    .map(|p| (de, Pid::from_raw(p)))
            })
        })
        .filter_map(|(de, pid)| {
            let mut path_buf = de.path();
            path_buf.push("stat");

            let mut s = String::new();
            let path = path_buf.as_path();
            match File::open(path).and_then(|mut f| f.read_to_string(&mut s)) {
                Ok(_) => {
                    if let Some(r) = s.split_whitespace().nth(3) {
                        match str::parse(r) {
                            Ok(p) => Some((pid, Pid::from_raw(p))),
                            _ => {
                                warn!("unable to interpret field 4 in {:?}", path);
                                None
                            }
                        }
                    } else {
                        warn!("unable to interpret {:?}", path);
                        None
                    }
                }
                Err(e) => {
                    warn!("unable to read {:?}: {}", path, e);
                    None
                }
            }
        })
        .filter_map(|(pid, ppid)| if ppid == parent { Some(pid) } else { None })
        .collect()
}

#[derive(Clone, Debug)]
enum OrphanState {
    BlissfulIgnorance(Pid),
    HasBeenSentSIGTERM(Pid),
    HasBeenSentSIGKILL(Pid, Instant),
    Errored(Pid, nix::Error),
    Carcass(Carcass),
}

fn transition_orphan(os: OrphanState) -> OrphanState {
    match os {
        OrphanState::BlissfulIgnorance(pid) => {
            info!("sending SIGTERM to orphan (pid={})", pid);
            match kill(pid, Some(SIGTERM)) {
                Ok(()) => OrphanState::HasBeenSentSIGTERM(pid),
                Err(e) => {
                    warn!("unable to send SIGTERM to orphan (pid={}): {}", pid, e);
                    OrphanState::Errored(pid, e)
                }
            }
        }
        OrphanState::HasBeenSentSIGTERM(pid) => {
            info!("sending SIGKILL to orphan (pid={})", pid);
            match kill(pid, Some(SIGKILL)) {
                Ok(()) => OrphanState::HasBeenSentSIGKILL(pid, Instant::now()),
                Err(e) => {
                    warn!("unable to send SIGKILL to orphan (pid={}): {}", pid, e);
                    OrphanState::Errored(pid, e)
                }
            }
        }
        OrphanState::HasBeenSentSIGKILL(pid, i) => {
            warn!(
                "orphan ({}) lingering (since {}s) after SIGKILL",
                pid,
                i.elapsed().as_secs()
            );
            os
        }
        os @ OrphanState::Carcass(_) => os,
        os @ OrphanState::Errored(_, _) => os,
    }
}

pub struct Reaper {
    orphans: HashMap<Pid, OrphanState>,
    trap: Trap,
}

impl Reaper {
    pub fn new() -> Self {
        Reaper {
            orphans: HashMap::new(),
            // it seems we need to set up the signal trap on the main thread, else it doesn't work
            trap: Trap::trap(&[SIGCHLD, SIGINT, SIGTERM]),
        }
    }

    pub fn spawn(mut self) {
        // signals to trap and handle
        // we only really care about SIGCHLD
        loop {
            let deadline = Instant::now() + Duration::from_secs(1);

            while let Some(signal) = self.trap.wait(deadline) {
                trace!("Caught signal {:?}", signal);
                match signal {
                    SIGCHLD => {
                        // received sigchld, try to get a carcass
                        if let Some(carcass) = reap() {
                            // got a dead process
                            match carcass {
                                // if the process exited normally, i.e. exit code 0, everything is fine
                                // if the process did not exit with 0, or it was signaled, kill all of its
                                // children
                                Carcass {
                                    pid,
                                    status: Some(0),
                                    signal: _,
                                } => {
                                    info!(
                                    "Reaped carcass of {}, exited with code 0, children can live",
                                    pid
                                );
                                }
                                Carcass {
                                    pid,
                                    status: Some(code),
                                    signal: _,
                                } => {
                                    info!(
                                    "Reaped carcass of {}, exited with code {}, killing children",
                                    pid, code
                                );
                                    self.mark_children(pid);
                                }
                                Carcass {
                                    pid,
                                    status: _,
                                    signal: Some(sig),
                                } => {
                                    info!(
                                        "Reaped {}, exited with signal {:?}, killing children",
                                        pid, sig
                                    );
                                    self.mark_children(pid);
                                }
                                _ => unreachable!(), // we always have either signal or status set
                            }
                            // remove pid from orphans if it exists
                            if self.orphans.contains_key(&carcass.pid) {
                                debug!("Reaped orphan (pid={})", carcass.pid);
                                self.orphans.remove(&carcass.pid);
                            }
                        }
                    }
                    s => debug!("Ignoring signal {:?}", s),
                }
            }

            // deadline expired
            trace!("transitioning orphans");
            self.transition_orphans();
        }
    }

    /// Mark and sweep all children of the given process ID. The children are gathered and signaled
    /// to exit.
    fn mark_children(&mut self, pid: Pid) {
        let children = list_children(pid);
        for child in &children {
            let _ = self
                .orphans
                .insert(*child, OrphanState::BlissfulIgnorance(*child));
        }
        trace!("Marked {} children for termination", children.len());
        self.transition_orphans();
    }

    fn transition_orphans(&mut self) {
        for orphan_state in self.orphans.values_mut() {
            *orphan_state = transition_orphan(orphan_state.to_owned());
        }

        trace!("Transitioned {} orphans", self.orphans.len());
    }
}
