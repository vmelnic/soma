"""
SOMA Body Discovery — POW 1.

Scans the target system and builds a function catalog.
This IS the body discovery phase from Whitepaper Section 5.1.

The catalog is DATA, not code. It describes what the body can do.
The bridge uses it to call ANY function the model requests.
"""

import ctypes
import ctypes.util
import platform
from dataclasses import dataclass


# ============================================================================
# macOS ARM64 struct definitions (discovered from target system headers)
# These describe the shape of the body's data — like register layouts.
# ============================================================================

class Timespec(ctypes.Structure):
    _fields_ = [
        ("tv_sec", ctypes.c_long),
        ("tv_nsec", ctypes.c_long),
    ]


class StatResult(ctypes.Structure):
    _fields_ = [
        ("st_dev", ctypes.c_int32),
        ("st_mode", ctypes.c_uint16),
        ("st_nlink", ctypes.c_uint16),
        ("st_ino", ctypes.c_uint64),
        ("st_uid", ctypes.c_uint32),
        ("st_gid", ctypes.c_uint32),
        ("st_rdev", ctypes.c_int32),
        ("st_atimespec", Timespec),
        ("st_mtimespec", Timespec),
        ("st_ctimespec", Timespec),
        ("st_birthtimespec", Timespec),
        ("st_size", ctypes.c_int64),
        ("st_blocks", ctypes.c_int64),
        ("st_blksize", ctypes.c_int32),
        ("st_flags", ctypes.c_uint32),
        ("st_gen", ctypes.c_uint32),
        ("st_lspare", ctypes.c_int32),
        ("st_qspare", ctypes.c_int64 * 2),
    ]


class Dirent(ctypes.Structure):
    _fields_ = [
        ("d_ino", ctypes.c_uint64),
        ("d_seekoff", ctypes.c_uint64),
        ("d_reclen", ctypes.c_uint16),
        ("d_namlen", ctypes.c_uint16),
        ("d_type", ctypes.c_uint8),
        ("d_name", ctypes.c_char * 1024),
    ]


class Utsname(ctypes.Structure):
    _fields_ = [
        ("sysname", ctypes.c_char * 256),
        ("nodename", ctypes.c_char * 256),
        ("release", ctypes.c_char * 256),
        ("version", ctypes.c_char * 256),
        ("machine", ctypes.c_char * 256),
    ]


class Timeval(ctypes.Structure):
    _fields_ = [
        ("tv_sec", ctypes.c_long),
        ("tv_usec", ctypes.c_int32),
    ]


# macOS fcntl constants (discovered from target headers)
O_RDONLY = 0x0000
O_WRONLY = 0x0001
O_CREAT = 0x0200
O_TRUNC = 0x0400

# Special control IDs (not libc calls)
EMIT_ID = -1
STOP_ID = -2


@dataclass
class CallingConvention:
    """A discovered way to invoke a libc function.
    The model picks a convention by ID. The bridge calls it generically."""
    id: int
    name: str
    description: str
    function: str           # libc function name
    fixed_args: dict        # constant args filled by the bridge
    var_args: list           # [{"name": ..., "type": ...}] filled by the model
    returns: str            # "fd", "ptr", "int", "bytes", "string",
                            # "struct_stat", "struct_dirent_list",
                            # "struct_utsname", "struct_timeval"
    auto_buffer_size: int   # if >0, bridge allocates a buffer


def discover_body() -> tuple[list[CallingConvention], object]:
    """Discover the target body's capabilities.
    Returns (catalog, libc_handle)."""

    libc_path = ctypes.util.find_library("c")
    libc = ctypes.CDLL(libc_path)

    catalog = []

    def add(name, desc, func, fixed, var_args, returns, buf=0):
        catalog.append(CallingConvention(
            id=len(catalog), name=name, description=desc,
            function=func, fixed_args=fixed, var_args=var_args,
            returns=returns, auto_buffer_size=buf,
        ))

    # --- File I/O (discovered from POSIX interface) ---
    add("open_read", "Open file for reading",
        "open", {"flags": O_RDONLY},
        [{"name": "path", "type": "string"}], "fd")

    add("create_file", "Create or truncate file for writing",
        "creat", {"mode": 0o644},
        [{"name": "path", "type": "string"}], "fd")

    add("read_content", "Read content from file descriptor",
        "read", {"count": 65536},
        [{"name": "fd", "type": "fd"}], "bytes", buf=65536)

    add("write_content", "Write data to file descriptor",
        "write", {},
        [{"name": "fd", "type": "fd"}, {"name": "data", "type": "bytes"}], "int")

    add("close_fd", "Close a file descriptor",
        "close", {},
        [{"name": "fd", "type": "fd"}], "int")

    # --- Directory (discovered from POSIX interface) ---
    add("open_dir", "Open directory for reading entries",
        "opendir", {},
        [{"name": "path", "type": "string"}], "ptr")

    add("read_dir_entries", "Read all entries from open directory",
        "readdir", {},
        [{"name": "dirp", "type": "ptr"}], "struct_dirent_list")

    add("close_dir", "Close directory handle",
        "closedir", {},
        [{"name": "dirp", "type": "ptr"}], "int")

    # --- File system operations ---
    add("delete_file", "Delete a file by path",
        "unlink", {},
        [{"name": "path", "type": "string"}], "int")

    add("create_dir", "Create a new directory",
        "mkdir", {"mode": 0o755},
        [{"name": "path", "type": "string"}], "int")

    add("rename_path", "Rename or move a file",
        "rename", {},
        [{"name": "old_path", "type": "string"},
         {"name": "new_path", "type": "string"}], "int")

    add("check_access", "Check if file exists and is accessible",
        "access", {"mode": 0},
        [{"name": "path", "type": "string"}], "int")

    add("file_stat", "Get file size and modification time",
        "stat", {},
        [{"name": "path", "type": "string"}], "struct_stat")

    # --- System information ---
    add("get_cwd", "Get current working directory",
        "getcwd", {"size": 1024},
        [], "string", buf=1024)

    add("get_time", "Get current time of day",
        "gettimeofday", {},
        [], "struct_timeval")

    add("get_uname", "Get operating system information",
        "uname", {},
        [], "struct_utsname")

    # Verify all functions exist on this target
    verified = []
    for conv in catalog:
        try:
            getattr(libc, conv.function)
            verified.append(conv)
        except AttributeError:
            pass

    for i, conv in enumerate(verified):
        conv.id = i

    return verified, libc


def catalog_summary(catalog: list[CallingConvention]) -> list[dict]:
    """Human-readable summary for proprioception."""
    return [
        {"id": c.id, "name": c.name, "description": c.description,
         "libc": c.function, "args": [a["name"] for a in c.var_args]}
        for c in catalog
    ]


if __name__ == "__main__":
    catalog, libc = discover_body()
    print(f"Discovered {len(catalog)} calling conventions "
          f"on {platform.system()} {platform.machine()}:\n")
    for c in catalog:
        args = ", ".join(a["name"] for a in c.var_args)
        print(f"  [{c.id:2d}] {c.name:20s} -> libc.{c.function}({args})")
