use std::process::Command;

pub struct PersistentCommand<'a> {
    cmd: &'a str,
    args: &'a str,

    restart_on_success: bool,
    restart_on_error: bool,
    restart_on_signal: bool,

    spawn_limit: Option<usize>,
    spawns: usize,
}

impl<'a> PersistentCommand<'a> {
    pub const fn new(cmd: &'a str, args: &'a str) -> Self {
        PersistentCommand {
            cmd,
            args,

            restart_on_success: false,
            restart_on_error: false,
            restart_on_signal: false,

            spawn_limit: None,
            spawns: 0,
        }
    }

    pub fn restart_on_success(mut self, restart: bool) -> Self {
        self.restart_on_success = restart;
        self
    }

    pub fn restart_on_error(mut self, restart: bool) -> Self {
        self.restart_on_error = restart;
        self
    }

    pub fn restart_on_signal(mut self, restart: bool) -> Self {
        self.restart_on_signal = restart;
        self
    }

    pub fn spawn_limit(mut self, limit: usize) -> Self {
        self.spawn_limit = Some(limit);
        self
    }

    pub(crate) fn spawn(
        &mut self,
        previous_exit_reason: Option<Event>,
    ) -> Result<u32, PersistentCommandError> {
        debug!("Creating command from persistent command");

        // In case there is an exit from a previous process, check if we need to respawn
        if let Some(reason) = previous_exit_reason {
            match reason {
                Event::ExitSuccess if !self.restart_on_success => {
                    debug!("Not respawning successful command");
                    return Err(PersistentCommandError::MustNotRespawn(reason));
                }
                Event::ExitCode if !self.restart_on_error => {
                    debug!("Not respawning errored command");
                    return Err(PersistentCommandError::MustNotRespawn(reason));
                }
                Event::ExitSignal if !self.restart_on_signal => {
                    debug!("Not respawning signaled command");
                    return Err(PersistentCommandError::MustNotRespawn(reason));
                }
                _ => (),
            }
        }

        if let Some(limit) = self.spawn_limit {
            if self.spawns >= limit {
                debug!(
                    "Command has ben spawned as much as allowed ({}), ignoring",
                    limit
                );
                return Err(PersistentCommandError::SpawnLimitReached(limit));
            }
        }

        self.spawns += 1;
        trace!("Command has been spawned {} times now", self.spawns);

        let mut cmd = Command::new(self.cmd);
        cmd.args(self.args.split_whitespace());

        let id = cmd.spawn().map(|child| child.id())?;

        Ok(id)
    }
}

impl<'a> std::fmt::Display for PersistentCommand<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} {}", self.cmd, self.args)
    }
}

#[derive(Debug)]
pub enum PersistentCommandError {
    SpawnLimitReached(usize),
    SpawnFailed(std::io::Error),
    MustNotRespawn(Event),
}

impl std::fmt::Display for PersistentCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PersistentCommandError::SpawnLimitReached(x) => {
                write!(f, "Spawn limit ({}) reached", x)
            }
            PersistentCommandError::SpawnFailed(e) => write!(f, "Spawning command failed: {}", e),
            PersistentCommandError::MustNotRespawn(e) => write!(
                f,
                "Previous command died due to {:?}, no need to respawn",
                e
            ),
        }
    }
}

impl std::error::Error for PersistentCommandError {}

impl From<std::io::Error> for PersistentCommandError {
    fn from(e: std::io::Error) -> Self {
        PersistentCommandError::SpawnFailed(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    ExitSuccess,
    ExitCode,
    ExitSignal,
}
