//! Child process lifecycle management for plugins that spawn external programs
//! (e.g., MCP Bridge plugin). All managed processes are killed on drop.

use anyhow::Result;
use std::collections::HashMap;
use std::process::Stdio;

/// A child process with captured stdio handles, owned by a plugin.
#[allow(dead_code)] // Spec feature: MCP Bridge plugin child processes
pub struct ManagedProcess {
    pub name: String,
    pub child: std::process::Child,
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
}

/// Named process registry. Ensures all spawned processes are cleaned up on drop.
#[allow(dead_code)] // Spec feature: MCP Bridge plugin process management
pub struct ProcessManager {
    processes: HashMap<String, ManagedProcess>,
}

#[allow(dead_code)] // Spec feature: MCP Bridge plugin process management
impl ProcessManager {
    pub fn new() -> Self {
        Self { processes: HashMap::new() }
    }

    /// Spawn a named child process with piped stdio and optional environment variables.
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

    /// Kill and remove a managed process by name.
    pub fn kill(&mut self, name: &str) -> Result<()> {
        if let Some(mut proc) = self.processes.remove(name) {
            proc.child.kill()?;
            tracing::info!(name = %name, "Child process killed");
        }
        Ok(())
    }

    /// Kill all managed processes. Called during shutdown and by the `Drop` impl.
    pub fn kill_all(&mut self) {
        let names: Vec<String> = self.processes.keys().cloned().collect();
        for name in names {
            let _ = self.kill(&name);
        }
    }

    /// Check if a named process is still running (non-blocking wait).
    pub fn is_running(&mut self, name: &str) -> bool {
        if let Some(proc) = self.processes.get_mut(name) {
            matches!(proc.child.try_wait(), Ok(None))
        } else {
            false
        }
    }

    /// List names of all managed processes (may include already-exited ones).
    pub fn list(&self) -> Vec<&str> {
        self.processes.keys().map(std::string::String::as_str).collect()
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        self.kill_all();
    }
}
