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
        ];
        Self { conventions }
    }
}

fn arg(name: &str, arg_type: &str, required: bool, desc: &str) -> super::interface::ArgSpec {
    super::interface::ArgSpec {
        name: name.to_string(),
        arg_type: arg_type.to_string(),
        required,
        description: desc.to_string(),
    }
}

fn conv(id: u32, name: &str, desc: &str, args: Vec<super::interface::ArgSpec>, ret: &str,
        latency_ms: u32, cleanup: Option<u32>) -> Convention {
    Convention {
        id,
        name: name.to_string(),
        description: desc.to_string(),
        call_pattern: "builtin".to_string(),
        args,
        return_type: ret.to_string(),
        estimated_latency_ms: latency_ms,
        cleanup_convention: cleanup,
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
                Ok(Value::List(entries))
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
                Ok(Value::Map(vec![
                    ("size".into(), stat.st_size.to_string()),
                    ("mode".into(), format!("{:o}", stat.st_mode)),
                    ("modified".into(), stat.st_mtime.to_string()),
                ]))
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
                Ok(Value::Map(vec![
                    ("system".into(), sysname),
                    ("machine".into(), machine),
                    ("release".into(), release),
                ]))
            }
            _ => Err(PluginError::NotFound(format!("unknown convention: {}", conv_id))),
        }
    }
}
