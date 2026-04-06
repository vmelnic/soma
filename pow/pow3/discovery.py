"""
SOMA Body Discovery — POW 1 (data-driven).

Scans the target system and builds a function catalog.
Each entry carries its ctypes signature AND call pattern as DATA.
The bridge uses the patterns generically — no function name dispatch.

Call patterns (6 total, like CPU addressing modes):
  direct       — marshal args, call, return raw result
  buffered_read — alloc buffer, call, return decoded buffer content
  write_bytes  — second arg is data with length
  struct_query — alloc struct, pass pointer, extract fields
  iterate      — loop call until NULL, collect d_name entries
  buffered_str — alloc string buffer, return decoded string
"""

import ctypes
import ctypes.util
import platform
from dataclasses import dataclass, field


# ============================================================================
# Struct definitions (discovered from target system headers)
# ============================================================================

class Timespec(ctypes.Structure):
    _fields_ = [("tv_sec", ctypes.c_long), ("tv_nsec", ctypes.c_long)]

class StatResult(ctypes.Structure):
    _fields_ = [
        ("st_dev", ctypes.c_int32), ("st_mode", ctypes.c_uint16),
        ("st_nlink", ctypes.c_uint16), ("st_ino", ctypes.c_uint64),
        ("st_uid", ctypes.c_uint32), ("st_gid", ctypes.c_uint32),
        ("st_rdev", ctypes.c_int32),
        ("st_atimespec", Timespec), ("st_mtimespec", Timespec),
        ("st_ctimespec", Timespec), ("st_birthtimespec", Timespec),
        ("st_size", ctypes.c_int64), ("st_blocks", ctypes.c_int64),
        ("st_blksize", ctypes.c_int32), ("st_flags", ctypes.c_uint32),
        ("st_gen", ctypes.c_uint32), ("st_lspare", ctypes.c_int32),
        ("st_qspare", ctypes.c_int64 * 2),
    ]

class Dirent(ctypes.Structure):
    _fields_ = [
        ("d_ino", ctypes.c_uint64), ("d_seekoff", ctypes.c_uint64),
        ("d_reclen", ctypes.c_uint16), ("d_namlen", ctypes.c_uint16),
        ("d_type", ctypes.c_uint8), ("d_name", ctypes.c_char * 1024),
    ]

class Utsname(ctypes.Structure):
    _fields_ = [
        ("sysname", ctypes.c_char * 256), ("nodename", ctypes.c_char * 256),
        ("release", ctypes.c_char * 256), ("version", ctypes.c_char * 256),
        ("machine", ctypes.c_char * 256),
    ]

class Timeval(ctypes.Structure):
    _fields_ = [("tv_sec", ctypes.c_long), ("tv_usec", ctypes.c_int32)]


O_RDONLY = 0x0000
EMIT_ID = -1
STOP_ID = -2


@dataclass
class CallingConvention:
    """A discovered way to invoke a libc function.
    Carries ALL information the bridge needs — signature, pattern, parsing.
    The bridge never looks up function names. It reads these fields."""
    id: int
    name: str
    description: str
    function: str
    var_args: list                   # filled by the model
    fixed_args: dict                 # filled by the bridge from catalog

    # ctypes signature (DATA, not code)
    call_pattern: str                # "direct", "buffered_read", "write_bytes",
                                     # "struct_query", "iterate", "buffered_str"
    c_argtypes: list                 # ctypes type list for the function
    c_restype: object                # ctypes return type

    # Pattern-specific data
    buffer_size: int = 0             # for buffered patterns
    struct_class: object = None      # for struct_query and iterate
    struct_fields: list = field(default_factory=list)  # [("key", "field", "transform")]
    result_eq_zero: bool = False     # if True, return (result == 0) as bool
    null_arg: bool = False           # if True, append NULL as extra arg (gettimeofday)


def discover_body():
    libc_path = ctypes.util.find_library("c")
    libc = ctypes.CDLL(libc_path)
    c = []

    def add(**kw):
        kw.setdefault("fixed_args", {})
        kw.setdefault("var_args", [])
        kw.setdefault("buffer_size", 0)
        kw.setdefault("struct_class", None)
        kw.setdefault("struct_fields", [])
        kw.setdefault("result_eq_zero", False)
        kw.setdefault("null_arg", False)
        kw["id"] = len(c)
        c.append(CallingConvention(**kw))

    # --- File I/O ---
    add(name="open_read", description="Open file for reading",
        function="open", var_args=[{"name": "path", "type": "string"}],
        fixed_args={"flags": O_RDONLY},
        call_pattern="direct",
        c_argtypes=[ctypes.c_char_p, ctypes.c_int],
        c_restype=ctypes.c_int)

    add(name="create_file", description="Create or truncate file for writing",
        function="creat", var_args=[{"name": "path", "type": "string"}],
        fixed_args={"mode": 0o644},
        call_pattern="direct",
        c_argtypes=[ctypes.c_char_p, ctypes.c_uint16],
        c_restype=ctypes.c_int)

    add(name="read_content", description="Read content from file descriptor",
        function="read", var_args=[{"name": "fd", "type": "fd"}],
        fixed_args={"count": 65536},
        call_pattern="buffered_read",
        c_argtypes=[ctypes.c_int, ctypes.c_void_p, ctypes.c_size_t],
        c_restype=ctypes.c_ssize_t,
        buffer_size=65536)

    add(name="write_content", description="Write data to file descriptor",
        function="write", var_args=[{"name": "fd", "type": "fd"}, {"name": "data", "type": "bytes"}],
        call_pattern="write_bytes",
        c_argtypes=[ctypes.c_int, ctypes.c_char_p, ctypes.c_size_t],
        c_restype=ctypes.c_ssize_t)

    add(name="close_fd", description="Close a file descriptor",
        function="close", var_args=[{"name": "fd", "type": "fd"}],
        call_pattern="direct",
        c_argtypes=[ctypes.c_int], c_restype=ctypes.c_int)

    # --- Directory ---
    add(name="open_dir", description="Open directory for reading entries",
        function="opendir", var_args=[{"name": "path", "type": "string"}],
        call_pattern="direct",
        c_argtypes=[ctypes.c_char_p], c_restype=ctypes.c_void_p)

    add(name="read_dir_entries", description="Read all directory entries",
        function="readdir", var_args=[{"name": "dirp", "type": "ptr"}],
        call_pattern="iterate",
        c_argtypes=[ctypes.c_void_p], c_restype=ctypes.POINTER(Dirent),
        struct_class=Dirent)

    add(name="close_dir", description="Close directory handle",
        function="closedir", var_args=[{"name": "dirp", "type": "ptr"}],
        call_pattern="direct",
        c_argtypes=[ctypes.c_void_p], c_restype=ctypes.c_int)

    # --- File system ---
    add(name="delete_file", description="Delete a file by path",
        function="unlink", var_args=[{"name": "path", "type": "string"}],
        call_pattern="direct",
        c_argtypes=[ctypes.c_char_p], c_restype=ctypes.c_int)

    add(name="create_dir", description="Create a new directory",
        function="mkdir", var_args=[{"name": "path", "type": "string"}],
        fixed_args={"mode": 0o755},
        call_pattern="direct",
        c_argtypes=[ctypes.c_char_p, ctypes.c_uint16], c_restype=ctypes.c_int)

    add(name="rename_path", description="Rename or move a file",
        function="rename",
        var_args=[{"name": "old_path", "type": "string"}, {"name": "new_path", "type": "string"}],
        call_pattern="direct",
        c_argtypes=[ctypes.c_char_p, ctypes.c_char_p], c_restype=ctypes.c_int)

    add(name="check_access", description="Check if file exists and is accessible",
        function="access", var_args=[{"name": "path", "type": "string"}],
        fixed_args={"mode": 0},
        call_pattern="direct",
        c_argtypes=[ctypes.c_char_p, ctypes.c_int], c_restype=ctypes.c_int,
        result_eq_zero=True)

    add(name="file_stat", description="Get file size and modification time",
        function="stat", var_args=[{"name": "path", "type": "string"}],
        call_pattern="struct_query",
        c_argtypes=[ctypes.c_char_p, ctypes.POINTER(StatResult)],
        c_restype=ctypes.c_int,
        struct_class=StatResult,
        struct_fields=[("size", "st_size", "raw"),
                       ("modified", "st_mtimespec", "timestamp"),
                       ("mode", "st_mode", "oct")])

    # --- System ---
    add(name="get_cwd", description="Get current working directory",
        function="getcwd", fixed_args={"size": 1024},
        call_pattern="buffered_str",
        c_argtypes=[ctypes.c_char_p, ctypes.c_size_t],
        c_restype=ctypes.c_char_p,
        buffer_size=1024)

    add(name="get_time", description="Get current time of day",
        function="gettimeofday",
        call_pattern="struct_query",
        c_argtypes=[ctypes.POINTER(Timeval), ctypes.c_void_p],
        c_restype=ctypes.c_int,
        struct_class=Timeval, null_arg=True,
        struct_fields=[("time", "tv_sec", "isotime")])

    add(name="get_uname", description="Get operating system information",
        function="uname",
        call_pattern="struct_query",
        c_argtypes=[ctypes.POINTER(Utsname)],
        c_restype=ctypes.c_int,
        struct_class=Utsname,
        struct_fields=[("system", "sysname", "decode"),
                       ("release", "release", "decode"),
                       ("machine", "machine", "decode")])

    # --- Network (Synaptic Protocol) ---
    # SEND is a body capability: the SOMA's body includes network access.
    # The model learns to compose programs ending with SEND instead of EMIT
    # when the intent involves another SOMA.
    add(name="send_signal", description="Send data to a peer SOMA via Synaptic Protocol",
        function="__synapse_send__",  # handled by bridge, not libc
        var_args=[{"name": "peer", "type": "string"}, {"name": "data", "type": "bytes"}],
        call_pattern="synapse_send",
        c_argtypes=[], c_restype=None)

    # Verify libc functions exist (skip synapse — it's not in libc)
    verified = []
    for conv in c:
        if conv.call_pattern == "synapse_send":
            verified.append(conv)  # synapse is always available
        else:
            try:
                getattr(libc, conv.function)
                verified.append(conv)
            except AttributeError:
                pass
    for i, conv in enumerate(verified):
        conv.id = i

    return verified, libc


def catalog_summary(catalog):
    return [{"id": c.id, "name": c.name, "description": c.description,
             "libc": c.function, "args": [a["name"] for a in c.var_args]}
            for c in catalog]


if __name__ == "__main__":
    catalog, libc = discover_body()
    print(f"Discovered {len(catalog)} conventions on {platform.system()} {platform.machine()}:\n")
    for c in catalog:
        args = ", ".join(a["name"] for a in c.var_args)
        print(f"  [{c.id:2d}] {c.name:20s} [{c.call_pattern:13s}] libc.{c.function}({args})")
