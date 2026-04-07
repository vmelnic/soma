//! Built-in plugin: POSIX file/system operations via libc.
//!
//! One line per operation. Zero domain logic.
//! The mind decides what to call. This plugin just executes.

use super::interface::{Convention, PluginError, SomaPlugin, Value};
use std::ffi::CString;

pub struct PosixPlugin {
    conventions: Vec<Convention>,
}

impl PosixPlugin {
    pub fn new() -> Self {
        let conventions = vec![
            // File operations — cleanup convention 4 (close_fd) for open handles
            conv(0,  "open_read",       "Open file for reading",
                vec![arg("path", "string", true, "File path")],
                "handle", 1, Some(4)),  // cleanup: close_fd
            conv(1,  "create_file",     "Create/truncate file for writing",
                vec![arg("path", "string", true, "File path")],
                "handle", 1, Some(4)),  // cleanup: close_fd
            conv(2,  "read_content",    "Read content from fd",
                vec![arg("fd", "handle", true, "File descriptor")],
                "string", 5, None),
            conv(3,  "write_content",   "Write data to fd",
                vec![arg("fd", "handle", true, "File descriptor"),
                     arg("data", "string", true, "Data to write")],
                "int", 5, None),
            conv(4,  "close_fd",        "Close file descriptor",
                vec![arg("fd", "handle", true, "File descriptor")],
                "int", 1, None),
            // Directory operations — cleanup convention 7 (close_dir) for open handles
            conv(5,  "open_dir",        "Open directory",
                vec![arg("path", "string", true, "Directory path")],
                "handle", 1, Some(7)),  // cleanup: close_dir
            conv(6,  "read_dir_entries","Read directory entries",
                vec![arg("dirp", "handle", true, "Directory handle")],
                "list", 10, None),
            conv(7,  "close_dir",       "Close directory",
                vec![arg("dirp", "handle", true, "Directory handle")],
                "int", 1, None),
            conv(8,  "delete_file",     "Delete file",
                vec![arg("path", "string", true, "File path")],
                "int", 1, None),
            conv(9,  "create_dir",      "Create directory",
                vec![arg("path", "string", true, "Directory path")],
                "int", 1, None),
            conv(10, "rename_path",     "Rename/move file",
                vec![arg("old", "string", true, "Source path"),
                     arg("new", "string", true, "Destination path")],
                "int", 1, None),
            conv(11, "check_access",    "Check file exists",
                vec![arg("path", "string", true, "File path")],
                "bool", 1, None),
            conv(12, "file_stat",       "Get file metadata",
                vec![arg("path", "string", true, "File path")],
                "map", 2, None),
            conv(13, "get_cwd",         "Get working directory",
                vec![], "string", 1, None),
            conv(14, "get_time",        "Get current time",
                vec![], "string", 1, None),
            conv(15, "get_uname",       "Get system info",
                vec![], "map", 1, None),
            // High-level filesystem operations (Catalog Section 4)
            conv(16, "read_file",       "Read entire file contents",
                vec![arg("path", "string", true, "File path")],
                "string", 10, None),
            conv(17, "write_file",      "Write content to file (create/overwrite)",
                vec![arg("path", "string", true, "File path"),
                     arg("content", "string", true, "Content to write")],
                "void", 10, None),
            conv(18, "list_dir_simple", "List directory contents (single call)",
                vec![arg("path", "string", true, "Directory path")],
                "list", 15, None),
            conv(19, "copy_file",       "Copy file from source to destination",
                vec![arg("from", "string", true, "Source path"),
                     arg("to", "string", true, "Destination path")],
                "void", 15, None),
            conv(20, "read_chunk",      "Read N bytes from file handle",
                vec![arg("fd", "handle", true, "File descriptor"),
                     arg("size", "int", true, "Bytes to read")],
                "bytes", 5, None),
            conv(21, "append_file",     "Append content to file",
                vec![arg("path", "string", true, "File path"),
                     arg("content", "string", true, "Content to append")],
                "void", 10, None),
        ];
        Self { conventions }
    }
}

fn arg(name: &str, arg_type: &str, required: bool, desc: &str) -> super::interface::ArgSpec {
    use super::interface::ArgType;
    let at = match arg_type {
        "int" => ArgType::Int,
        "float" => ArgType::Float,
        "bool" => ArgType::Bool,
        "bytes" => ArgType::Bytes,
        "handle" => ArgType::Handle,
        "any" => ArgType::Any,
        _ => ArgType::String, // "string" and fallback
    };
    super::interface::ArgSpec {
        name: name.to_string(),
        arg_type: at,
        required,
        description: desc.to_string(),
    }
}

fn conv(id: u32, name: &str, desc: &str, args: Vec<super::interface::ArgSpec>, ret: &str,
        latency_ms: u32, cleanup: Option<u32>) -> Convention {
    use super::interface::{ReturnSpec, CleanupSpec, SideEffect};
    let returns = match ret {
        "handle" => ReturnSpec::Handle,
        "string" => ReturnSpec::Value("string".into()),
        "int" => ReturnSpec::Value("int".into()),
        "bool" => ReturnSpec::Value("bool".into()),
        "list" => ReturnSpec::Value("list".into()),
        "map" => ReturnSpec::Value("map".into()),
        "bytes" => ReturnSpec::Value("bytes".into()),
        _ => ReturnSpec::Void,
    };
    // Conventions with side effects: create/delete/rename/write/mkdir + high-level write/copy/append
    let side_effects = match id {
        1 | 3 | 8 | 9 | 10 | 17 | 19 | 21 => vec![SideEffect("filesystem".into())],
        _ => vec![],
    };
    // Deterministic: stat, access, cwd, uname are deterministic-ish; reads depend on state
    let is_deterministic = matches!(id, 11 | 13);
    Convention {
        id,
        name: name.to_string(),
        description: desc.to_string(),
        call_pattern: "builtin".to_string(),
        args,
        returns,
        is_deterministic,
        estimated_latency_ms: latency_ms,
        max_latency_ms: 30000,
        side_effects,
        cleanup: cleanup.map(|cid| CleanupSpec { convention_id: cid, pass_result_as: 0 }),
    }
}

fn to_cstring(val: &Value) -> Result<CString, PluginError> {
    match val {
        Value::String(s) => CString::new(s.as_bytes())
            .map_err(|_| PluginError::InvalidArg("invalid path".into())),
        _ => Err(PluginError::InvalidArg("expected string".into())),
    }
}

fn get_fd(val: &Value) -> Result<i32, PluginError> {
    match val {
        Value::Handle(h) => Ok(*h as i32),
        Value::Int(n) => Ok(*n as i32),
        _ => Err(PluginError::InvalidArg("expected handle/fd".into())),
    }
}

impl SomaPlugin for PosixPlugin {
    fn name(&self) -> &str { "posix" }

    fn permissions(&self) -> super::interface::PluginPermissions {
        super::interface::PluginPermissions {
            filesystem: vec!["/".to_string()],
            network: vec![],
            env_vars: vec!["HOME".to_string(), "PATH".to_string()],
            process_spawn: false,
        }
    }

    fn conventions(&self) -> Vec<Convention> {
        self.conventions.clone()
    }

    fn execute(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match conv_id {
            // open_read
            0 => {
                let path = to_cstring(&args[0])?;
                let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY) };
                if fd < 0 { return Err(PluginError::NotFound(format!("file not found: {}", args[0]))); }
                Ok(Value::Handle(fd as u64))
            }
            // create_file (creat)
            1 => {
                let path = to_cstring(&args[0])?;
                let fd = unsafe { libc::creat(path.as_ptr(), 0o644) };
                if fd < 0 { return Err(PluginError::Failed("create failed".into())); }
                Ok(Value::Handle(fd as u64))
            }
            // read_content
            2 => {
                let fd = get_fd(&args[0])?;
                let mut buf = vec![0u8; 65536];
                let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                if n < 0 { return Err(PluginError::Failed("read failed".into())); }
                buf.truncate(n as usize);
                Ok(Value::String(String::from_utf8_lossy(&buf).to_string()))
            }
            // write_content
            3 => {
                let fd = get_fd(&args[0])?;
                let data = match &args[1] {
                    Value::String(s) => s.as_bytes().to_vec(),
                    Value::Bytes(b) => b.clone(),
                    _ => return Err(PluginError::InvalidArg("expected string/bytes".into())),
                };
                let n = unsafe { libc::write(fd, data.as_ptr() as *const libc::c_void, data.len()) };
                Ok(Value::Int(n as i64))
            }
            // close_fd
            4 => {
                let fd = get_fd(&args[0])?;
                unsafe { libc::close(fd); }
                Ok(Value::Int(0))
            }
            // open_dir
            5 => {
                let path = to_cstring(&args[0])?;
                let dirp = unsafe { libc::opendir(path.as_ptr()) };
                if dirp.is_null() { return Err(PluginError::NotFound(format!("dir not found: {}", args[0]))); }
                Ok(Value::Handle(dirp as u64))
            }
            // read_dir_entries
            6 => {
                let dirp = match &args[0] {
                    Value::Handle(h) => *h as *mut libc::DIR,
                    _ => return Err(PluginError::InvalidArg("expected handle".into())),
                };
                let mut entries = Vec::new();
                loop {
                    let entry = unsafe { libc::readdir(dirp) };
                    if entry.is_null() { break; }
                    let name = unsafe {
                        std::ffi::CStr::from_ptr((*entry).d_name.as_ptr())
                            .to_string_lossy().to_string()
                    };
                    if name != "." && name != ".." {
                        entries.push(name);
                    }
                }
                entries.sort();
                Ok(Value::List(entries.into_iter().map(Value::String).collect()))
            }
            // close_dir
            7 => {
                let dirp = match &args[0] {
                    Value::Handle(h) => *h as *mut libc::DIR,
                    _ => return Err(PluginError::InvalidArg("expected handle".into())),
                };
                unsafe { libc::closedir(dirp); }
                Ok(Value::Int(0))
            }
            // delete_file
            8 => {
                let path = to_cstring(&args[0])?;
                let rc = unsafe { libc::unlink(path.as_ptr()) };
                if rc != 0 { return Err(PluginError::NotFound(format!("file not found: {}", args[0]))); }
                Ok(Value::Int(0))
            }
            // create_dir
            9 => {
                let path = to_cstring(&args[0])?;
                unsafe { libc::mkdir(path.as_ptr(), 0o755); }
                Ok(Value::Int(0))
            }
            // rename_path
            10 => {
                let old = to_cstring(&args[0])?;
                let new = to_cstring(&args[1])?;
                let rc = unsafe { libc::rename(old.as_ptr(), new.as_ptr()) };
                if rc != 0 { return Err(PluginError::Failed("rename failed".into())); }
                Ok(Value::Int(0))
            }
            // check_access
            11 => {
                let path = to_cstring(&args[0])?;
                let rc = unsafe { libc::access(path.as_ptr(), libc::F_OK) };
                Ok(Value::Bool(rc == 0))
            }
            // file_stat
            12 => {
                let path = to_cstring(&args[0])?;
                let mut stat: libc::stat = unsafe { std::mem::zeroed() };
                let rc = unsafe { libc::stat(path.as_ptr(), &mut stat) };
                if rc != 0 { return Err(PluginError::NotFound(format!("stat failed: {}", args[0]))); }
                Ok(Value::Map(std::collections::HashMap::from([
                    ("size".to_string(), Value::Int(stat.st_size as i64)),
                    ("mode".to_string(), Value::String(format!("{:o}", stat.st_mode))),
                    ("modified".to_string(), Value::Int(stat.st_mtime as i64)),
                ])))
            }
            // get_cwd
            13 => {
                let mut buf = vec![0u8; 1024];
                let ptr = unsafe { libc::getcwd(buf.as_mut_ptr() as *mut i8, buf.len()) };
                if ptr.is_null() { return Err(PluginError::Failed("getcwd failed".into())); }
                let cwd = unsafe { std::ffi::CStr::from_ptr(ptr).to_string_lossy().to_string() };
                Ok(Value::String(cwd))
            }
            // get_time
            14 => {
                let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
                unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()); }
                // Format as ISO timestamp
                let secs = tv.tv_sec;
                Ok(Value::String(format!("timestamp:{}", secs)))
            }
            // get_uname
            15 => {
                let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
                unsafe { libc::uname(&mut uts); }
                let sysname = unsafe { std::ffi::CStr::from_ptr(uts.sysname.as_ptr()).to_string_lossy().to_string() };
                let machine = unsafe { std::ffi::CStr::from_ptr(uts.machine.as_ptr()).to_string_lossy().to_string() };
                let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()).to_string_lossy().to_string() };
                Ok(Value::Map(std::collections::HashMap::from([
                    ("system".to_string(), Value::String(sysname)),
                    ("machine".to_string(), Value::String(machine)),
                    ("release".to_string(), Value::String(release)),
                ])))
            }
            // High-level filesystem operations (Catalog Section 4)
            // read_file — single-call: reads entire file to string
            16 => {
                let path = match &args[0] { Value::String(s) => s.clone(), _ => return Err(PluginError::InvalidArg("expected string".into())) };
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| PluginError::NotFound(format!("read_file failed: {}", e)))?;
                Ok(Value::String(content))
            }
            // write_file — single-call: create/overwrite file
            17 => {
                let path = match &args[0] { Value::String(s) => s.clone(), _ => return Err(PluginError::InvalidArg("expected string".into())) };
                let content = match &args[1] { Value::String(s) => s.clone(), _ => return Err(PluginError::InvalidArg("expected string".into())) };
                std::fs::write(&path, &content)
                    .map_err(|e| PluginError::Failed(format!("write_file failed: {}", e)))?;
                Ok(Value::Null)
            }
            // list_dir_simple — single-call: list directory entries sorted
            18 => {
                let path = match &args[0] { Value::String(s) => s.clone(), _ => return Err(PluginError::InvalidArg("expected string".into())) };
                let mut entries = Vec::new();
                for entry in std::fs::read_dir(&path).map_err(|e| PluginError::NotFound(format!("{}", e)))? {
                    if let Ok(e) = entry {
                        let name = e.file_name().to_string_lossy().to_string();
                        entries.push(Value::String(name));
                    }
                }
                entries.sort_by(|a, b| format!("{}", a).cmp(&format!("{}", b)));
                Ok(Value::List(entries))
            }
            // copy_file — single-call: copy file
            19 => {
                let from = match &args[0] { Value::String(s) => s.clone(), _ => return Err(PluginError::InvalidArg("expected string".into())) };
                let to = match &args[1] { Value::String(s) => s.clone(), _ => return Err(PluginError::InvalidArg("expected string".into())) };
                std::fs::copy(&from, &to).map_err(|e| PluginError::Failed(format!("copy failed: {}", e)))?;
                Ok(Value::Null)
            }
            // read_chunk — read N bytes from fd (low-level handle, high-level size)
            20 => {
                let fd = get_fd(&args[0])?;
                let size = match &args[1] { Value::Int(n) => *n as usize, _ => return Err(PluginError::InvalidArg("expected int".into())) };
                let mut buf = vec![0u8; size];
                let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, size) };
                if n < 0 { return Err(PluginError::Failed("read_chunk failed".into())); }
                buf.truncate(n as usize);
                Ok(Value::Bytes(buf))
            }
            // append_file — single-call: append to file (create if missing)
            21 => {
                let path = match &args[0] { Value::String(s) => s.clone(), _ => return Err(PluginError::InvalidArg("expected string".into())) };
                let content = match &args[1] { Value::String(s) => s.clone(), _ => return Err(PluginError::InvalidArg("expected string".into())) };
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().append(true).create(true).open(&path)
                    .map_err(|e| PluginError::Failed(format!("append failed: {}", e)))?;
                file.write_all(content.as_bytes())
                    .map_err(|e| PluginError::Failed(format!("append write failed: {}", e)))?;
                Ok(Value::Null)
            }
            _ => Err(PluginError::NotFound(format!("unknown convention: {}", conv_id))),
        }
    }
}
