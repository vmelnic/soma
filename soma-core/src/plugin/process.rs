//! Plugin child process management (for MCP Bridge and similar plugins).
//! Allows plugins to spawn and manage external processes.

use anyhow::Result;
use std::collections::HashMap;
use std::process::Stdio;

/// A managed child process spawned by a plugin.
#[allow(dead_code)] // Spec feature: MCP Bridge plugin child processes
pub struct ManagedProcess {
    pub name: String,
    pub child: std::process::Child,
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
}

/// Manager for plugin child processes.
#[allow(dead_code)] // Spec feature: MCP Bridge plugin process management
pub struct ProcessManager {
    processes: HashMap<String, ManagedProcess>,
}

#[allow(dead_code)] // Spec feature: MCP Bridge plugin process management
impl ProcessManager {
    pub fn new() -> Self {
        Self { processes: HashMap::new() }
    }

    /// Spawn a child process.
    pub fn spawn(
        &mut self,
        name: &str,
        command: &str,
        args: &[&str],
        env: &HashMap<String, String>,
    ) -> Result<&ManagedProcess> {
        let mut cmd = std::process::Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();

        tracing::info!(name = %name, command = %command, "Child process spawned");

        let managed = ManagedProcess {
            name: name.to_string(),
            child,
            stdin,
            stdout,
        };

        let key = name.to_string();
        self.processes.insert(key.clone(), managed);
        Ok(self.processes.get(&key).unwrap())
    }

    /// Kill a child process.
    pub fn kill(&mut self, name: &str) -> Result<()> {
        if let Some(mut proc) = self.processes.remove(name) {
            proc.child.kill()?;
            tracing::info!(name = %name, "Child process killed");
        }
        Ok(())
    }

    /// Kill all child processes.
    pub fn kill_all(&mut self) {
        let names: Vec<String> = self.processes.keys().cloned().collect();
        for name in names {
            let _ = self.kill(&name);
        }
    }

    /// Check if a process is running.
    pub fn is_running(&mut self, name: &str) -> bool {
        if let Some(proc) = self.processes.get_mut(name) {
            matches!(proc.child.try_wait(), Ok(None))
        } else {
            false
        }
    }

    /// List running processes.
    pub fn list(&self) -> Vec<&str> {
        self.processes.keys().map(std::string::String::as_str).collect()
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        self.kill_all();
    }
}
