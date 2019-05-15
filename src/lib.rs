#[macro_use]
extern crate log;

use std::collections::HashMap;
use std::fmt;
use std::fs::{read_dir, File};
use std::io::Read;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

use nix::sys::signal::kill;
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{getpid, Pid};

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
fn reap() -> Option<Carcass> {
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
    children: Vec<Pid>,
    trap: Trap,

    persistent_commands_map: HashMap<Pid, Cmd>,

    pid: Pid,
}

enum Event {
    ExitSuccess,
    ExitCode,
    ExitSignal,
}

struct Cmd(String, String);

impl Reaper {
    pub fn new() -> Self {
        Reaper {
            orphans: HashMap::new(),
            children: Vec::new(),
            // TODO document in reaper new that this must be called on the main thread
            trap: Trap::trap(&[SIGCHLD, SIGINT, SIGTERM]),

            persistent_commands_map: HashMap::new(),

            pid: getpid(),
        }
    }

    pub fn spawn(mut self, persistent_commands: &[(&str, &str)]) {
        for cmd in persistent_commands {
            self.spawn_child(cmd.0, cmd.1).unwrap();
        }
        let _ = self.new_children(); // make sure we know about these processes

        loop {
            let deadline = Instant::now() + Duration::from_secs(1);

            while let Some(signal) = self.trap.wait(deadline) {
                trace!("Caught signal {:?}", signal);
                match signal {
                    SIGCHLD => {
                        // received sigchld, try to get a carcass
                        if let Some(carcass) = reap() {
                            // got a dead process
                            let event = match carcass {
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
                                    Event::ExitSuccess
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
                                    Event::ExitCode
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
                                    Event::ExitSignal
                                }
                                _ => unreachable!(), // we always have either signal or status set
                            };

                            // get a list of children for this process
                            // this also forgets the current carcass pid as a child
                            let children = self.new_children();
                            debug!("Reaped process has {} children", children.len());

                            // see if the children need to be marked
                            match event {
                                Event::ExitCode | Event::ExitSignal => {
                                    self.mark_orphans(&children);
                                    self.ensure_process(&carcass.pid).unwrap();
                                }
                                Event::ExitSuccess => {
                                    // make sure forked processes have their pid updated
                                    if children.len() > 0 {
                                        self.update_ensured_process_pid(&carcass.pid, &children[0]);
                                    }
                                }
                            }

                            // finally remove pid from orphans if it exists
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
            self.transition_orphans();
        }
    }

    /// Mark and sweep all children of the given process ID. The children are gathered and signaled
    /// to exit.
    fn mark_orphans(&mut self, orphans: &[Pid]) {
        for child in orphans {
            let _ = self
                .orphans
                .insert(*child, OrphanState::BlissfulIgnorance(*child));
        }

        trace!("Marked {} children for termination", orphans.len());
        self.transition_orphans();
    }

    fn transition_orphans(&mut self) {
        for orphan_state in self.orphans.values_mut() {
            *orphan_state = transition_orphan(orphan_state.to_owned());
        }

        trace!("Transitioned {} orphans", self.orphans.len());
    }

    /// get a list of all new children since the last time this method is called, and remember
    /// all current children
    fn new_children(&mut self) -> Vec<Pid> {
        trace!("Finding children we don't know about yet");

        let all_children = list_children(self.pid);

        let new_children = all_children
            .iter()
            .filter(|p| !self.children.contains(p))
            .map(|p| *p)
            .collect();

        // remember the new children
        self.children = all_children;

        new_children
    }

    fn ensure_process(&mut self, pid: &Pid) -> Result<(), std::io::Error> {
        if let Some(cmd) = self.persistent_commands_map.remove(pid) {
            self.spawn_child(&cmd.0, &cmd.1)?;
        }
        Ok(())
    }

    fn update_ensured_process_pid(&mut self, pid: &Pid, new_pid: &Pid) {
        if let Some(cmd) = self.persistent_commands_map.remove(pid) {
            let _ = self.persistent_commands_map.insert(*new_pid, cmd);
        }
    }

    fn spawn_child(&mut self, cmd: &str, args: &str) -> Result<(), std::io::Error> {
        debug!("Spawning child {}", cmd);

        let mut command = Command::new(cmd);
        command.args(args.split_whitespace());

        let id = command.spawn().map(|child| child.id())?;

        info!("Spawned child {} {} (pid={})", cmd, args, id);

        self.persistent_commands_map.insert(
            Pid::from_raw(id as i32),
            Cmd(String::from(cmd), String::from(args)),
        );
        Ok(())
    }
}
